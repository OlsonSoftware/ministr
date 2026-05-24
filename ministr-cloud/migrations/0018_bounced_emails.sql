-- F3.1b-ii-c — track email addresses that bounced so future invite
-- sends can warn the operator. Populated by the Resend bounce webhook.
CREATE TABLE IF NOT EXISTS bounced_emails (
    email       TEXT        PRIMARY KEY,
    bounced_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    reason      TEXT        NOT NULL DEFAULT ''
);
