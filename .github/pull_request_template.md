## What and why
<!-- One or two sentences. What does this change, and why does it matter to players? -->

## How to try it
<!-- Steps a reviewer (or future you) can follow to see it working. -->

## Checklist
- [ ] `cargo fmt --all`, `cargo clippy ... -D warnings` and `cargo test` pass locally (CI will check too)
- [ ] New behaviour has a test (or a note here on why not)
- [ ] Database change? It is a **new** numbered file in `migrations/` (never edit an applied one) and works on a database that already has data
- [ ] New setting? Added to `.env.example`, `render.yaml` and `DEPLOY.md`
- [ ] No secrets, tokens or personal data in the diff

## After it merges
<!-- Anything to do or watch once it is live: a Render variable to add, a Discord setting, a feature to try. -->
