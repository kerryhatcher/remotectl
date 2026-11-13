//! Packet cryptography for the planned LoRa link. Not yet connected to either radio.
//!
//! The receiver must create a fresh unpredictable 16-byte challenge on every session,
//! and the sender must use a unique 24-byte CSPRNG nonce for every packet under a key.
//! Reusing either across sessions can defeat replay protection or confidentiality.

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{AeadInPlace, KeyInit},
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

const DOMAIN: &[u8] = b"remotectl-input-v1";
const HEADER: usize = 49; // version, challenge, sequence, nonce
const TAG: usize = 16;
const SIGNATURE: usize = 64;
pub const MAX_PACKET: usize = HEADER + 123 + TAG + SIGNATURE;

#[derive(Debug, PartialEq, Eq)]
pub enum PacketError {
    Invalid,
    Replay,
    BufferTooSmall,
}

/// Encrypt and sign a command. `nonce` must be generated afresh by a CSPRNG.
pub fn seal(
    key: &[u8; 32],
    signing_key: &SigningKey,
    challenge: &[u8; 16],
    sequence: u64,
    nonce: &[u8; 24],
    payload: &[u8],
    output: &mut [u8],
) -> Result<usize, PacketError> {
    if payload.len() > 123 || sequence == 0 {
        return Err(PacketError::Invalid);
    }
    let len = HEADER + payload.len() + TAG + SIGNATURE;
    if output.len() < len {
        return Err(PacketError::BufferTooSmall);
    }
    output[0] = 1;
    output[1..17].copy_from_slice(challenge);
    output[17..25].copy_from_slice(&sequence.to_be_bytes());
    output[25..49].copy_from_slice(nonce);
    let (header, rest) = output[..len].split_at_mut(HEADER);
    let (body, rest) = rest.split_at_mut(payload.len());
    body.copy_from_slice(payload);
    let cipher = XChaCha20Poly1305::new(key.into());
    let tag = cipher
        .encrypt_in_place_detached(XNonce::from_slice(nonce), header, body)
        .map_err(|_| PacketError::Invalid)?;
    rest[..TAG].copy_from_slice(&tag);
    // Include a fixed domain in the signature, independent of the packet format.
    let signed_len = len - SIGNATURE;
    let mut signed = [0u8; DOMAIN.len() + MAX_PACKET - SIGNATURE];
    signed[..DOMAIN.len()].copy_from_slice(DOMAIN);
    signed[DOMAIN.len()..DOMAIN.len() + signed_len].copy_from_slice(&output[..signed_len]);
    let signature = signing_key.sign(&signed[..DOMAIN.len() + signed_len]);
    output[signed_len..len].copy_from_slice(&signature.to_bytes());
    Ok(len)
}

/// In-memory replay state for one fresh receiver-generated challenge.
pub struct Receiver {
    challenge: [u8; 16],
    last_sequence: u64,
}

impl Receiver {
    /// The caller must supply a fresh CSPRNG challenge, including after power loss.
    pub fn new(challenge: [u8; 16]) -> Self {
        Self {
            challenge,
            last_sequence: 0,
        }
    }

    /// Verify, decrypt, and validate a packet before advancing the replay window.
    pub fn open<'a>(
        &mut self,
        key: &[u8; 32],
        verifying_key: &VerifyingKey,
        packet: &[u8],
        plaintext: &'a mut [u8],
    ) -> Result<crate::InputCommand<'a>, PacketError> {
        if packet.len() < HEADER + TAG + SIGNATURE
            || packet.len() > MAX_PACKET
            || packet[0] != 1
            || packet[1..17] != self.challenge
        {
            return Err(PacketError::Invalid);
        }
        let sequence = u64::from_be_bytes(
            packet[17..25]
                .try_into()
                .map_err(|_| PacketError::Invalid)?,
        );
        if sequence <= self.last_sequence {
            return Err(PacketError::Replay);
        }
        let signed_len = packet.len() - SIGNATURE;
        let mut signed = [0u8; DOMAIN.len() + MAX_PACKET - SIGNATURE];
        signed[..DOMAIN.len()].copy_from_slice(DOMAIN);
        signed[DOMAIN.len()..DOMAIN.len() + signed_len].copy_from_slice(&packet[..signed_len]);
        let signature =
            Signature::from_slice(&packet[signed_len..]).map_err(|_| PacketError::Invalid)?;
        verifying_key
            .verify_strict(&signed[..DOMAIN.len() + signed_len], &signature)
            .map_err(|_| PacketError::Invalid)?;
        let body_len = signed_len - HEADER - TAG;
        if plaintext.len() < body_len {
            return Err(PacketError::BufferTooSmall);
        }
        plaintext[..body_len].copy_from_slice(&packet[HEADER..HEADER + body_len]);
        let cipher = XChaCha20Poly1305::new(key.into());
        let nonce = XNonce::from_slice(&packet[25..49]);
        let tag = chacha20poly1305::Tag::from_slice(&packet[HEADER + body_len..signed_len]);
        cipher
            .decrypt_in_place_detached(nonce, &packet[..HEADER], &mut plaintext[..body_len], tag)
            .map_err(|_| PacketError::Invalid)?;
        let command = crate::InputCommand::decode(&plaintext[..body_len])
            .map_err(|_| PacketError::Invalid)?;
        self.last_sequence = sequence;
        Ok(command)
    }
}

/// Acknowledgments are authenticated and bound to the receiver's fresh challenge.
pub const ACK_LEN: usize = 7 + 16 + 1 + 32;
const ACK_DOMAIN: &[u8] = b"remotectl-ack-v1";
const ACK_PREFIX: &[u8; 7] = b"RCTACK1";

pub fn make_ack(key: &[u8; 32], challenge: &[u8; 16], status: u8) -> [u8; ACK_LEN] {
    use hmac::{Hmac, Mac};
    let mut out = [0u8; ACK_LEN];
    out[..7].copy_from_slice(ACK_PREFIX);
    out[7..23].copy_from_slice(challenge);
    out[23] = status;
    let mut mac =
        <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(ACK_DOMAIN);
    mac.update(&out[..24]);
    out[24..].copy_from_slice(&mac.finalize().into_bytes());
    out
}

pub fn verify_ack(key: &[u8; 32], challenge: &[u8; 16], packet: &[u8]) -> Result<u8, PacketError> {
    use hmac::{Hmac, Mac};
    if packet.len() != ACK_LEN || &packet[..7] != ACK_PREFIX || &packet[7..23] != challenge {
        return Err(PacketError::Invalid);
    }
    let mut mac =
        <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(ACK_DOMAIN);
    mac.update(&packet[..24]);
    mac.verify_slice(&packet[24..])
        .map_err(|_| PacketError::Invalid)?;
    Ok(packet[23])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputCommand;

    #[test]
    fn acknowledgments_require_the_key_challenge_and_untampered_status() {
        let key = [9; 32];
        let challenge = [3; 16];
        let ack = make_ack(&key, &challenge, 0);
        assert_eq!(verify_ack(&key, &challenge, &ack), Ok(0));
        assert_eq!(
            verify_ack(&[8; 32], &challenge, &ack),
            Err(PacketError::Invalid)
        );
        assert_eq!(verify_ack(&key, &[4; 16], &ack), Err(PacketError::Invalid));
        let mut altered = ack;
        altered[23] = 5;
        assert_eq!(
            verify_ack(&key, &challenge, &altered),
            Err(PacketError::Invalid)
        );
        assert_eq!(
            verify_ack(&key, &challenge, &ack[..ack.len() - 1]),
            Err(PacketError::Invalid)
        );
    }

    #[test]
    fn all_mouse_actions_survive_serial_framing_encryption_and_signature() {
        use crate::{MouseButton, serial};
        let actions = [
            InputCommand::Move {
                dx: i16::MIN,
                dy: i16::MAX,
            },
            InputCommand::Click {
                button: MouseButton::Left,
                double: false,
            },
            InputCommand::Click {
                button: MouseButton::Right,
                double: true,
            },
            InputCommand::Click {
                button: MouseButton::Middle,
                double: false,
            },
            InputCommand::Scroll { amount: -127 },
            InputCommand::Scroll { amount: 127 },
        ];
        let signing = SigningKey::from_bytes(&[7; 32]);
        let key = [3; 32];
        for (index, action) in actions.iter().enumerate() {
            let challenge = [index as u8 + 1; 16];
            let nonce = [index as u8 + 1; 24];
            let mut serial_frame = [0; serial::MAX_FRAME];
            let serial_len = serial::encode(action, &mut serial_frame).unwrap();
            assert_eq!(
                serial::decode(&serial_frame[..serial_len]).unwrap(),
                *action
            );
            let mut packet = [0; MAX_PACKET];
            let packet_len = seal(
                &key,
                &signing,
                &challenge,
                1,
                &nonce,
                &serial_frame[serial::HEADER_LEN..serial_len],
                &mut packet,
            )
            .unwrap();
            let mut receiver = Receiver::new(challenge);
            let mut output = [0; serial::MAX_PAYLOAD];
            assert_eq!(
                receiver
                    .open(
                        &key,
                        &signing.verifying_key(),
                        &packet[..packet_len],
                        &mut output
                    )
                    .unwrap(),
                *action
            );
            assert_eq!(
                receiver.open(
                    &key,
                    &signing.verifying_key(),
                    &packet[..packet_len],
                    &mut output
                ),
                Err(PacketError::Replay)
            );
        }
    }

    #[test]
    fn rejects_tampering_wrong_keys_replays_and_old_sessions() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let wrong_signing = SigningKey::from_bytes(&[8; 32]);
        let key = [3; 32];
        let challenge = [4; 16];
        let command = InputCommand::Move { dx: -3, dy: 42 };
        let mut encoded = [0; 123];
        let n = command.encode(&mut encoded).unwrap();
        let mut packet = [0; MAX_PACKET];
        let size = seal(
            &key,
            &signing,
            &challenge,
            1,
            &[9; 24],
            &encoded[..n],
            &mut packet,
        )
        .unwrap();
        let mut output = [0; 123];
        let mut receiver = Receiver::new(challenge);
        let mut altered = packet;
        altered[HEADER] ^= 1;
        assert_eq!(
            receiver.open(
                &key,
                &signing.verifying_key(),
                &altered[..size],
                &mut output
            ),
            Err(PacketError::Invalid)
        );
        assert_eq!(
            receiver.open(
                &key,
                &wrong_signing.verifying_key(),
                &packet[..size],
                &mut output
            ),
            Err(PacketError::Invalid)
        );
        assert_eq!(
            receiver.open(
                &[5; 32],
                &signing.verifying_key(),
                &packet[..size],
                &mut output
            ),
            Err(PacketError::Invalid)
        );
        assert_eq!(
            receiver.open(&key, &signing.verifying_key(), &packet[..size], &mut output),
            Ok(command)
        );
        assert_eq!(
            receiver.open(&key, &signing.verifying_key(), &packet[..size], &mut output),
            Err(PacketError::Replay)
        );
        assert_eq!(
            Receiver::new([6; 16]).open(
                &key,
                &signing.verifying_key(),
                &packet[..size],
                &mut output
            ),
            Err(PacketError::Invalid)
        );
    }
}
