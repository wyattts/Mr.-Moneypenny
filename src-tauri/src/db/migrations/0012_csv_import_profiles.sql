-- Mr. Moneypenny migration 0012: CSV import profiles.
--
-- A "profile" is the saved column-mapping recipe for one bank's CSV
-- export format. Different banks order columns differently and use
-- different date formats; once the user maps it, we save the recipe so
-- subsequent imports of the same bank's export skip the mapping screen.
--
-- `header_signature` is a stable hash of the normalized column-header
-- row (lowercased, whitespace-trimmed). When the user picks a new CSV
-- file, we hash its header row and look up a matching profile. Match →
-- auto-suggest the profile (user can override). Miss → mapping screen.
--
-- `mapping_json` payload shape (validated server-side, opaque to SQLite):
--   {
--     "date_col": int,
--     "amount_col": int,
--     "merchant_col": int,
--     "description_col": int | null,
--     "category_col": int | null,
--     "date_format": string,           // e.g., "MM/DD/YYYY"
--     "neg_means_refund": bool,
--     "skip_rows": int                 // header rows to skip
--   }
--
-- Forward-only; bumps user_version to 12.

CREATE TABLE csv_import_profiles (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  header_signature TEXT,
  mapping_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_used_at TEXT
);

CREATE INDEX idx_csv_import_profiles_signature
  ON csv_import_profiles(header_signature);

PRAGMA user_version = 12;
