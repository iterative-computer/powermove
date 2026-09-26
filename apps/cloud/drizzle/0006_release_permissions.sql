ALTER TABLE "extensions" ADD COLUMN "permissions" jsonb DEFAULT '[]'::jsonb NOT NULL;--> statement-breakpoint
ALTER TABLE "releases" ADD COLUMN "permissions" jsonb DEFAULT '[]'::jsonb NOT NULL;