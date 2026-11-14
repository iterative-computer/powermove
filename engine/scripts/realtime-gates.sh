#!/bin/sh
# Real-time deadline tests. They are `#[ignore]`d in the default run because a
# loaded, shared machine cannot honour them; their correctness halves run by
# default. Run this on an otherwise idle Mac before a release: every test must pass.
set -eu
cd "$(dirname "$0")/.."
cargo +1.96.1 test --locked -p pm-audio --test playback -- --ignored --exact --test-threads=1 \
  cold_starts_of_sixteen_distinct_voices_meet_the_first_buffer_deadline \
  a_scheduled_play_keeps_its_line_on_a_real_time_device
