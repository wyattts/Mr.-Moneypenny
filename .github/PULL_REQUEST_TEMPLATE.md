## Summary

<!-- 1-3 sentences on what this PR does and why. -->

## Related issue

<!-- Closes #N, or "n/a". -->

## Changes

<!-- Bullet list of significant changes. -->

## Privacy / security checklist

- [ ] No new outbound network calls (or new calls are documented and disclosed in `docs/privacy.md`).
- [ ] No new analytics, telemetry, or auto-uploaded crash reports.
- [ ] Secrets (API keys, bot tokens) continue to live in OS keychain only.
- [ ] If schema changed: migration is forward-only and tested.
- [ ] If LLM tool dispatcher changed: tool schemas validated; destructive ops still confirm.

## Test plan

- [ ] `cargo test` passes locally
- [ ] `npm run typecheck` and `npm run lint` pass locally
- [ ] Manually exercised the affected flow

## Sign-off

- [ ] Commits are signed off (DCO) — see `CONTRIBUTING.md`.
