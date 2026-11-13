#!/usr/bin/env python3
"""Create a private, sourceable key file for one remotectl link."""
import argparse
import os
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("path", type=Path, help="new key file (must not already exist)")
args = parser.parse_args()
key = Ed25519PrivateKey.generate()
private = key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption()).hex()
public = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
fd = os.open(args.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w", encoding="ascii") as output:
    output.write(f"export REMOTECTL_LINK_KEY={os.urandom(32).hex()}\n")
    output.write(f"export REMOTECTL_SIGN_KEY={private}\n")
    output.write(f"export REMOTECTL_VERIFY_KEY={public}\n")
print(f"Created {args.path} with owner-only permissions; keep it private.")
