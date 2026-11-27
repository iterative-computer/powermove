ALTER TYPE "public"."credit_kind" ADD VALUE 'hold';--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "fresh_input_tokens" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "cache_write_long_tokens" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "reasoning_tokens" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "cost_nano_usd" bigint DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "hold_nano_usd" bigint DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "hold_expires_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "output_cap" integer;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "usage_source" text;--> statement-breakpoint
ALTER TABLE "credit_ledger" ADD COLUMN "price_version" text;