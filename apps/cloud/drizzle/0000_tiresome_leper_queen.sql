CREATE TYPE "public"."moderation" AS ENUM('none', 'hidden', 'removed');--> statement-breakpoint
CREATE TYPE "public"."object_gc_state" AS ENUM('live', 'claimed');--> statement-breakpoint
CREATE TYPE "public"."visibility" AS ENUM('public', 'unlisted');--> statement-breakpoint
CREATE TABLE "desktop_auth" (
	"state" text PRIMARY KEY NOT NULL,
	"challenge" text NOT NULL,
	"token_hash" text,
	"expires_at" timestamp with time zone NOT NULL,
	"consumed_at" timestamp with time zone
);
--> statement-breakpoint
CREATE TABLE "extensions" (
	"repo_id" uuid PRIMARY KEY NOT NULL,
	"handle" text NOT NULL,
	"slug" text NOT NULL,
	"name" text NOT NULL,
	"tagline" text NOT NULL,
	"about" text,
	"category" text NOT NULL,
	"icon_key" text,
	"latest_release_id" uuid,
	"install_count" integer DEFAULT 0 NOT NULL,
	"fork_count" integer DEFAULT 0 NOT NULL,
	"licence" text DEFAULT 'MIT' NOT NULL,
	"search" "tsvector" GENERATED ALWAYS AS (to_tsvector('simple', coalesce(name,'') || ' ' || coalesce(tagline,'') || ' ' || coalesce(slug,'') || ' ' || coalesce(handle,''))) STORED
);
--> statement-breakpoint
CREATE TABLE "featured" (
	"repo_id" uuid NOT NULL,
	"section" text NOT NULL,
	"position" integer NOT NULL,
	CONSTRAINT "featured_section_position" UNIQUE("section","position")
);
--> statement-breakpoint
CREATE TABLE "installs" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" text NOT NULL,
	"repo_id" uuid NOT NULL,
	"release_id" uuid NOT NULL,
	"installed_at" timestamp with time zone DEFAULT now() NOT NULL,
	"removed_at" timestamp with time zone
);
--> statement-breakpoint
CREATE TABLE "moderation_log" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"repo_id" uuid NOT NULL,
	"release_id" uuid,
	"action" text NOT NULL,
	"reason" text NOT NULL,
	"actor" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "object_leases" (
	"user_id" text NOT NULL,
	"sha" text NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	CONSTRAINT "object_leases_user_id_sha_pk" PRIMARY KEY("user_id","sha")
);
--> statement-breakpoint
CREATE TABLE "objects" (
	"sha" text PRIMARY KEY NOT NULL,
	"type" text NOT NULL,
	"size" integer NOT NULL,
	"sha256" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_seen_at" timestamp with time zone DEFAULT now() NOT NULL,
	"gc_state" "object_gc_state" DEFAULT 'live' NOT NULL
);
--> statement-breakpoint
CREATE TABLE "publish_counter" (
	"user_id" text NOT NULL,
	"hour" timestamp with time zone NOT NULL,
	"count" integer DEFAULT 0 NOT NULL,
	CONSTRAINT "publish_counter_user_id_hour_pk" PRIMARY KEY("user_id","hour")
);
--> statement-breakpoint
CREATE TABLE "publishers" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"handle" text NOT NULL,
	"user_id" text,
	"claimed_at" timestamp with time zone DEFAULT now() NOT NULL,
	"tombstoned_at" timestamp with time zone,
	CONSTRAINT "publishers_handle_unique" UNIQUE("handle")
);
--> statement-breakpoint
CREATE TABLE "refs" (
	"repo_id" uuid NOT NULL,
	"name" text NOT NULL,
	"sha" text NOT NULL,
	CONSTRAINT "refs_repo_id_name_pk" PRIMARY KEY("repo_id","name")
);
--> statement-breakpoint
CREATE TABLE "release_objects" (
	"release_id" uuid NOT NULL,
	"sha" text NOT NULL,
	CONSTRAINT "release_objects_release_id_sha_pk" PRIMARY KEY("release_id","sha")
);
--> statement-breakpoint
CREATE TABLE "releases" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"repo_id" uuid NOT NULL,
	"version" text NOT NULL,
	"commit_sha" text NOT NULL,
	"tree_sha" text NOT NULL,
	"tar_key" text NOT NULL,
	"tar_sha256" text NOT NULL,
	"manifest" jsonb NOT NULL,
	"notes" text,
	"based_on_release_id" uuid,
	"api_version" integer NOT NULL,
	"file_count" integer NOT NULL,
	"size_bytes" bigint NOT NULL,
	"scan_waivers" jsonb DEFAULT '[]'::jsonb NOT NULL,
	"yanked_at" timestamp with time zone,
	"published_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "releases_repo_version" UNIQUE("repo_id","version")
);
--> statement-breakpoint
CREATE TABLE "reports" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"repo_id" uuid NOT NULL,
	"reporter_id" text,
	"reason" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "repos" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"owner_id" uuid NOT NULL,
	"slug" text NOT NULL,
	"visibility" "visibility" DEFAULT 'public' NOT NULL,
	"moderation" "moderation" DEFAULT 'none' NOT NULL,
	"forked_from_repo_id" uuid,
	"forked_from_release_id" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"tombstoned_at" timestamp with time zone,
	CONSTRAINT "repos_owner_slug" UNIQUE("owner_id","slug")
);
--> statement-breakpoint
CREATE TABLE "user_settings" (
	"user_id" text PRIMARY KEY NOT NULL,
	"remember_installs" boolean DEFAULT true NOT NULL
);
--> statement-breakpoint
CREATE TABLE "account" (
	"id" text PRIMARY KEY NOT NULL,
	"account_id" text NOT NULL,
	"provider_id" text NOT NULL,
	"user_id" text NOT NULL,
	"access_token" text,
	"refresh_token" text,
	"id_token" text,
	"access_token_expires_at" timestamp with time zone,
	"refresh_token_expires_at" timestamp with time zone,
	"scope" text,
	"password" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "session" (
	"id" text PRIMARY KEY NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"token" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"ip_address" text,
	"user_agent" text,
	"user_id" text NOT NULL,
	CONSTRAINT "session_token_unique" UNIQUE("token")
);
--> statement-breakpoint
CREATE TABLE "user" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"email" text NOT NULL,
	"email_verified" boolean DEFAULT false NOT NULL,
	"image" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"username" text,
	"display_username" text,
	CONSTRAINT "user_email_unique" UNIQUE("email"),
	CONSTRAINT "user_username_unique" UNIQUE("username")
);
--> statement-breakpoint
CREATE TABLE "verification" (
	"id" text PRIMARY KEY NOT NULL,
	"identifier" text NOT NULL,
	"value" text NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "extensions" ADD CONSTRAINT "extensions_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "extensions" ADD CONSTRAINT "extensions_latest_release_id_releases_id_fk" FOREIGN KEY ("latest_release_id") REFERENCES "public"."releases"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "featured" ADD CONSTRAINT "featured_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "installs" ADD CONSTRAINT "installs_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "installs" ADD CONSTRAINT "installs_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "installs" ADD CONSTRAINT "installs_release_id_releases_id_fk" FOREIGN KEY ("release_id") REFERENCES "public"."releases"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_log" ADD CONSTRAINT "moderation_log_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "moderation_log" ADD CONSTRAINT "moderation_log_release_id_releases_id_fk" FOREIGN KEY ("release_id") REFERENCES "public"."releases"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "object_leases" ADD CONSTRAINT "object_leases_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "object_leases" ADD CONSTRAINT "object_leases_sha_objects_sha_fk" FOREIGN KEY ("sha") REFERENCES "public"."objects"("sha") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "publish_counter" ADD CONSTRAINT "publish_counter_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "publishers" ADD CONSTRAINT "publishers_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "refs" ADD CONSTRAINT "refs_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "release_objects" ADD CONSTRAINT "release_objects_release_id_releases_id_fk" FOREIGN KEY ("release_id") REFERENCES "public"."releases"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "release_objects" ADD CONSTRAINT "release_objects_sha_objects_sha_fk" FOREIGN KEY ("sha") REFERENCES "public"."objects"("sha") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "releases" ADD CONSTRAINT "releases_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "releases" ADD CONSTRAINT "releases_based_on_release_id_releases_id_fk" FOREIGN KEY ("based_on_release_id") REFERENCES "public"."releases"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_repo_id_repos_id_fk" FOREIGN KEY ("repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_reporter_id_user_id_fk" FOREIGN KEY ("reporter_id") REFERENCES "public"."user"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "repos" ADD CONSTRAINT "repos_owner_id_publishers_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."publishers"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "repos" ADD CONSTRAINT "repos_forked_from_repo_id_repos_id_fk" FOREIGN KEY ("forked_from_repo_id") REFERENCES "public"."repos"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "repos" ADD CONSTRAINT "repos_forked_from_release_id_releases_id_fk" FOREIGN KEY ("forked_from_release_id") REFERENCES "public"."releases"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "user_settings" ADD CONSTRAINT "user_settings_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "account" ADD CONSTRAINT "account_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "session" ADD CONSTRAINT "session_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "extensions_search_idx" ON "extensions" USING gin ("search");--> statement-breakpoint
CREATE UNIQUE INDEX "installs_active_unique" ON "installs" USING btree ("user_id","repo_id") WHERE "installs"."removed_at" is null;--> statement-breakpoint
CREATE INDEX "installs_user_idx" ON "installs" USING btree ("user_id");--> statement-breakpoint
CREATE UNIQUE INDEX "publishers_user_id_unique" ON "publishers" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "release_objects_sha_idx" ON "release_objects" USING btree ("sha");--> statement-breakpoint
CREATE INDEX "releases_repo_published_idx" ON "releases" USING btree ("repo_id","published_at");--> statement-breakpoint
CREATE INDEX "repos_forked_from_idx" ON "repos" USING btree ("forked_from_repo_id");--> statement-breakpoint
CREATE INDEX "account_user_idx" ON "account" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "session_user_idx" ON "session" USING btree ("user_id");--> statement-breakpoint
CREATE INDEX "verification_identifier_idx" ON "verification" USING btree ("identifier");