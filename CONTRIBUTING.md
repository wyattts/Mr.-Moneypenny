# Contributing to Mr. Moneypenny

Thanks for your interest in contributing. Mr. Moneypenny is FOSS under the AGPL-3.0, and contributions of all kinds are welcome — bug reports, code, docs, design, translations, threat-model review.

## Ground rules

- Be kind. The [Code of Conduct](CODE_OF_CONDUCT.md) applies everywhere this project is discussed.
- Privacy is the first-class design constraint. Any change that adds telemetry, analytics, third-party network calls, or weakens the local-only data posture will not be merged.
- Keep the dependency tree small. Every added crate or npm package is a supply-chain risk. Prefer the standard library and platform primitives where reasonable.

## Workflow

1. Open an issue first for anything non-trivial. For bugs, include steps to reproduce; for features, describe the user-visible behavior.
2. Fork the repo and create a topic branch off `main`: `git checkout -b feat/short-description` or `fix/short-description`.
3. Make your changes in small, reviewable commits.
4. Sign off your commits — see DCO below.
5. Open a pull request against `main`. Fill out the PR template.
6. CI must pass: lint, type-check, unit tests, and (for Rust) `cargo audit`.

## Developer Certificate of Origin (DCO)

This project uses the [Developer Certificate of Origin](https://developercertificate.org/) to track contribution authorship. Every commit must include a sign-off line:

```
Signed-off-by: Your Name <your.email@example.com>
```

Add it automatically with `git commit -s`. By signing off, you certify the contribution is yours to give under the project's license.

## Branch and commit style

- `main` is the integration branch. Keep it green.
- Topic branches: `feat/...`, `fix/...`, `docs/...`, `chore/...`, `refactor/...`.
- Commit messages: short imperative subject (≤72 chars), blank line, optional body explaining *why*.

## Code style

- **Rust:** `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass. Add `#![forbid(unsafe_code)]` to new crates unless explicitly justified.
- **TypeScript:** ESLint + Prettier configurations live in the repo. `tsc --strict` must pass.
- **SQL:** parameterized queries only — never string concatenation. The LLM must never see or generate SQL.
- Prefer small, focused functions over large ones. No premature abstraction.

## Testing

- Add tests for any non-trivial logic.
- Domain logic (period math, fixed/variable pacing, insights aggregation) must have unit tests. The LLM must never be a substitute for tested code.
- For Telegram and LLM provider code, use record/replay fixtures so CI doesn't need network access.

## Security-sensitive changes

If your change touches secret storage, network egress, the LLM tool dispatcher, the Telegram authorization flow, or the SQLite schema, please flag it in the PR description so it gets extra review. See also [SECURITY.md](SECURITY.md).

## License of contributions

By contributing, you agree your contributions will be licensed under the [GNU Affero General Public License v3.0](LICENSE).
