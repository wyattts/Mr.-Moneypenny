-- Mr. Moneypenny migration 0017: the "open hub" export view.
-- Forward-only. Bumps user_version to 17 on success.
--
-- v_ledger_v1 is a PUBLISHED, STABLE contract — the single shape that
-- the data-export feature reads from. Internal tables (expenses,
-- categories, telegram_authorized_chats) may be refactored freely; this
-- view absorbs the change so exports keep the same columns. If the
-- contract ever genuinely must change, add v_ledger_v2 and keep v1
-- working — same versioning discipline as the LLM tool schemas.
--
-- Read-only by construction (a view over LEFT JOINs). Money stays in
-- integer cents; `signed_amount_cents` is negative for refunds so
-- consumers don't have to re-derive the sign. Timestamps are the stored
-- ISO8601-UTC strings, unmodified.

CREATE VIEW v_ledger_v1 AS
SELECT
  e.id                                                                   AS expense_id,
  e.occurred_at                                                          AS occurred_at,
  e.created_at                                                           AS created_at,
  e.amount_cents                                                         AS amount_cents,
  CASE WHEN e.is_refund = 1 THEN -e.amount_cents ELSE e.amount_cents END  AS signed_amount_cents,
  e.currency                                                             AS currency,
  e.is_refund                                                            AS is_refund,
  e.refund_for_expense_id                                                AS refund_for_expense_id,
  c.name                                                                 AS category_name,
  c.kind                                                                 AS category_kind,
  c.monthly_target_cents                                                 AS category_monthly_target_cents,
  e.description                                                          AS description,
  e.source                                                               AS source,
  t.display_name                                                         AS logged_by
FROM expenses e
LEFT JOIN categories c ON c.id = e.category_id
LEFT JOIN telegram_authorized_chats t ON t.chat_id = e.logged_by_chat_id;

PRAGMA user_version = 17;
