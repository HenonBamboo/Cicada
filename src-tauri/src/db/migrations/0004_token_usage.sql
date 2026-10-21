-- Only request metadata and provider-reported usage; never prompts, keys or response text.
CREATE TABLE token_usage (
    id TEXT PRIMARY KEY NOT NULL,
    started_at TEXT NOT NULL,
    business_date TEXT NOT NULL,
    provider TEXT NOT NULL,
    origin_hash TEXT NOT NULL,
    model TEXT NOT NULL,
    purpose TEXT NOT NULL,
    request_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('success', 'failed', 'cancelled')),
    input_tokens INTEGER CHECK (input_tokens IS NULL OR (typeof(input_tokens) = 'integer' AND input_tokens >= 0)),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR (typeof(output_tokens) = 'integer' AND output_tokens >= 0)),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR (typeof(total_tokens) = 'integer' AND total_tokens >= 0))
);
CREATE INDEX idx_token_usage_business_date ON token_usage(business_date, started_at);
