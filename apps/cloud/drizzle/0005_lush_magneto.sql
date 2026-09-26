CREATE TABLE "abuse_counters" (
	"scope" text NOT NULL,
	"key" text NOT NULL,
	"window_start" timestamp with time zone NOT NULL,
	"count" integer DEFAULT 0 NOT NULL,
	CONSTRAINT "abuse_counters_scope_key_window_start_pk" PRIMARY KEY("scope","key","window_start")
);
--> statement-breakpoint
DROP TABLE "publish_counter" CASCADE;