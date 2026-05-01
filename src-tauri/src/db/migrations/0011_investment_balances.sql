-- Mr. Moneypenny migration 0011: investment starting balances.
--
-- The investment calculator (v0.3.0 forecast tools) needs to know the
-- current balance of each investing-kind category — the app tracks
-- contributions but not balances, so without this users with existing
-- accounts (e.g., a Roth IRA opened years ago) would get severely
-- under-projected futures.
--
-- Both columns are nullable; meaningful only for `kind = 'investing'`
-- categories the user has chosen to populate. The fixed/variable
-- categories simply ignore them.
--
-- Forward-only; bumps user_version to 11.

ALTER TABLE categories ADD COLUMN starting_balance_cents INTEGER
  CHECK (starting_balance_cents IS NULL OR starting_balance_cents >= 0);

ALTER TABLE categories ADD COLUMN balance_as_of TEXT;

PRAGMA user_version = 11;
