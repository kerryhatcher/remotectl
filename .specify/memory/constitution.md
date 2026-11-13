# remotectl Constitution

## Core Principles

### I. USB Recovery Is Mandatory

Every device firmware MUST provide and preserve a USB command that resets the device into
flashing or bootloader mode without physical button presses. Each supported board MUST have a
documented host command or script for entering that mode and reflashing. A network-only reset
endpoint does not satisfy this requirement. An unavailable, broken, or unverified recovery path
MUST be reported as unfinished work.

### II. Authenticated Remote Input

Remote keyboard and mouse commands MUST be encrypted and signed, and the receiving device MUST
verify authorization and reject modified or replayed commands before producing HID input. Keys
and credentials MUST NOT be committed to the repository. Authentication failures MUST NOT result
in input execution.

### III. Explicit Hardware Boundaries

Board-specific pin assignments and peripheral behavior MUST be documented and kept in the
corresponding firmware crate. Shared protocol and command definitions MUST remain usable in a
`no_std` environment. A board's capabilities MUST be verified before claiming support for them.

### IV. Verifiable Changes

Changes MUST be checked with tests or build checks appropriate to their scope. Protocol and
security changes MUST include tests for valid and invalid inputs, including relevant replay and
tampering cases. Firmware and USB changes MUST have their reset and reflashing path verified on
available affected hardware; unperformed hardware checks and their consequences MUST be reported.

### V. Accurate Operational Documentation

Documentation MUST distinguish implemented and verified behavior from planned or experimental
capabilities. User-facing instructions MUST identify required configuration, the affected device,
and the commands needed to build, operate, and recover it. Known limitations MUST be stated
without claiming untested reliability.

## System Constraints

- The intended roles are the Heltec LoRa sender and the XIAO USB HID receiver; changes to these
  roles MUST update the relevant specifications and documentation.
- The firmware targets ESP32-S3 hardware, while host tools build for the host environment.
- Existing plain HTTP endpoints MUST be documented as trusted-network-only until protected by a
  secure transport; credentials MUST be treated as secrets.

## Development and Verification

- Feature specifications and plans MUST identify the affected boards, protocol, security
  boundaries, and the USB recovery and verification approach.
- Changes spanning crates MUST keep their shared interfaces compatible or document the migration.
- Reviews MUST check the applicable build and test results, hardware verification, recovery
  path, and outstanding limitations. A missing hardware check MUST be recorded rather than
  represented as a pass.
- Do not flash hardware as part of routine build or test checks unless the task authorizes it;
  report whether hardware was actually flashed and tested.

## Governance

This constitution governs project specifications, plans, implementation, and reviews alongside
repository instructions. Amendments MUST record their rationale, update the version and amendment
date, and be reviewed and approved by the project owner. Conflicts and necessary migration work
MUST be identified in the amendment.

Constitution versions use semantic versioning: MAJOR for incompatible principle removal or
redefinition, MINOR for new principles or materially expanded requirements, and PATCH for
clarifications and non-semantic corrections. Reviews MUST assess compliance with applicable
principles and document unresolved exceptions and verification gaps.

**Version**: 1.0.0 | **Ratified**: 2026-11-13 | **Last Amended**: 2026-11-13
