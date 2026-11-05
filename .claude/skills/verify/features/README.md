# Wristper verification map

This map lists the user-facing Wristper behaviors that `scripts/verify-watch-e2e.sh` proves on a real Mac. Read [../SKILL.md](../SKILL.md) first for launch, safety, and cleanup.

## Baseline preconditions

- `Wristper Test.app` is built at `target/app/` and re-signed with the Developer ID.
- Wristper Test has Microphone and Accessibility, and `hex status --json` reports `permissions.ready: true`.
- The installed `/Applications/Wristper.app` and its data folder are never used.

## Features

- [Apple Watch dictation](./apple-watch.md) covers pairing, delivery, paste into TextEdit and Terminal, `/status`, resend, and Enter.
