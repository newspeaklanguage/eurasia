#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# M7 (newspeak/EURASIA_M7_SNAPSHOTS_REPLAY_DESIGN_2026-11-18.md): checkpoints,
# restore and replay.
# 1. The kernel tests, recorded on the command line with checkpoints, replay
#    to the same outputs from the start and from a checkpoint taken in the
#    middle of a turn; recording does not change the outcomes; a log with one
#    printed line altered diverges there (the negative control).
# 2. The debugger probe, whose broken thread is a chain of detached
#    activations kept across turns: its log replays from every checkpoint,
#    and each checkpoint, run on its own, prints the rest of the output.
# 3. Sessions recorded on wasm (the kernel tests, waiting for timers, and
#    the JS tests, with callbacks) replay on the command line.
# 4. A checkpoint restored into a new host (EurasiaRestoreProbe, on wasm): an
#    Alien in a lazy slot is looked up again, one in an ordinary slot is
#    dead, and the restore handler runs.
# Needs the images of scripts/kernel-tests.sh, scripts/reflection-tests.sh and
# scripts/web-tests.sh. Outputs go to target/replay/.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
P=$PRIMORDIALSOUP/newspeak
OUT=$EURASIA/target/replay
mkdir -p "$OUT"
for image in target/kerneltests/base.eus target/reflection/debugger/base.eus target/web/js/base.eus; do
  [ -f "$image" ] || { echo "$image is missing: run kernel-tests.sh, reflection-tests.sh and web-tests.sh first"; exit 1; }
done
cargo build -q --release -p eurasia-native
cargo build -q --release -p eurasia-web --target wasm32-unknown-unknown
E=$EURASIA/target/release/eurasia
WASM=$EURASIA/target/wasm32-unknown-unknown/release/eurasia_web.wasm
checkpoints() { "$E" --replay "$1" 999999 2>&1 | sed -n 's/.*has \([0-9]*\) checkpoints.*/\1/p'; }
replays() { "$E" --replay "$@" > "$OUT/replayed.txt" 2> "$OUT/replay.err" && grep -q "the replay matched the log" "$OUT/replay.err"; }

echo "== the kernel tests, recorded and replayed on the command line"
"$E" target/kerneltests/base.eus > "$OUT/kernel-plain.txt" 2>&1 || true
"$E" --record "$OUT/kernel.log" --checkpoint-every 500000 target/kerneltests/base.eus > "$OUT/kernel-recorded.txt" 2>&1 || true
# Outcomes only: how often a repeating timer fires depends on the clock.
grep '>>' "$OUT/kernel-plain.txt" | sort > "$OUT/kernel-plain-outcomes.txt"
grep '>>' "$OUT/kernel-recorded.txt" | sort > "$OUT/kernel-recorded-outcomes.txt"
cmp "$OUT/kernel-plain-outcomes.txt" "$OUT/kernel-recorded-outcomes.txt" || { echo "recording changed the outcomes"; exit 1; }
n=$(checkpoints "$OUT/kernel.log")
[ "$n" -ge 3 ] || { echo "too few checkpoints ($n)"; exit 1; }
replays "$OUT/kernel.log" || { cat "$OUT/replay.err"; exit 1; }
cmp "$OUT/kernel-recorded.txt" "$OUT/replayed.txt" || { echo "the replay printed differently"; exit 1; }
replays "$OUT/kernel.log" $((n / 2)) || { cat "$OUT/replay.err"; exit 1; }
echo "replayed from the start and from checkpoint $((n / 2)) of $((n - 1))"
perl -0pe 's/SimulationTests>>testSimulationValue3 success/SimulationTests>>testSimulationValue3 sUccess/' "$OUT/kernel.log" > "$OUT/tampered.log"
if "$E" --replay "$OUT/tampered.log" $((n - 1)) > /dev/null 2> "$OUT/tampered.err"; then
  echo "the tampered log replayed without diverging"; exit 1
fi
grep -q 'recorded: Print("SimulationTests>>testSimulationValue3 sUccess")' "$OUT/tampered.err" \
  || { cat "$OUT/tampered.err"; echo "the tampered log diverged elsewhere"; exit 1; }
echo "the tampered log diverges at the altered line"

echo "== the debugger probe: detached activations in checkpoints"
"$E" --record "$OUT/debugger.log" --checkpoint-every 5 target/reflection/debugger/base.eus > "$OUT/debugger.txt"
cmp "$OUT/debugger.txt" target/reflection/debugger/output.txt || { echo "recording changed the output"; exit 1; }
n=$(checkpoints "$OUT/debugger.log")
[ "$n" -ge 10 ] || { echo "too few checkpoints ($n)"; exit 1; }
k=0
while [ $k -lt "$n" ]; do
  replays "$OUT/debugger.log" $k || { echo "checkpoint $k:"; cat "$OUT/replay.err"; exit 1; }
  if [ $k -gt 0 ]; then
    "$E" --checkpoint "$OUT/debugger.log" $k "$OUT/checkpoint.eus"
    "$E" "$OUT/checkpoint.eus" > "$OUT/restored.txt"
    lines=$(wc -l < "$OUT/restored.txt")
    tail -n "$lines" "$OUT/debugger.txt" | cmp - "$OUT/restored.txt" \
      || { echo "checkpoint $k, restored, does not print the rest of the output"; exit 1; }
  fi
  k=$((k + 1))
done
echo "replayed from each of $n checkpoints; each restored on its own finishes the run"

echo "== sessions recorded on wasm, replayed on the command line"
EURASIA_RECORD="$OUT/kernel-wasm.log" EURASIA_RECORD_EVERY=500000 \
  node web/run-node.mjs "$WASM" target/kerneltests/base.eus > "$OUT/kernel-wasm.txt" 2>&1 || true
tail -1 "$OUT/kernel-wasm.txt" | grep -qx done || { echo "the kernel tests did not finish on wasm"; exit 1; }
replays "$OUT/kernel-wasm.log" || { cat "$OUT/replay.err"; exit 1; }
n=$(checkpoints "$OUT/kernel-wasm.log")
replays "$OUT/kernel-wasm.log" $((n - 1)) || { cat "$OUT/replay.err"; exit 1; }
grep "replayed" "$OUT/replay.err"
EURASIA_RECORD="$OUT/js.log" node web/run-node.mjs "$WASM" target/web/js/base.eus > "$OUT/js.txt" 2>&1 || true
replays "$OUT/js.log" || { cat "$OUT/replay.err"; exit 1; }
cmp "$OUT/js.txt" "$OUT/replayed.txt" || { echo "the JS tests replay printed differently"; exit 1; }
echo "the kernel tests (with checkpoints) and the JS tests replay"

echo "== a checkpoint restored into a new host"
mkdir -p "$OUT/restore"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel \
  "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaRestoreProbe.ns" \
  "$P/Kernel.ns" "$P/platform/PrimordialSoup/Collections.ns" "$NEWSPEAK/platform/Eurasia/Actors.ns" "$NEWSPEAK/platform/Eurasia/Runtime.ns" \
  "$OUT/restore/genesis.eus" > "$OUT/restore/package.log"
(cd "$OUT/restore" && "$E" genesis.eus base.eus)
EURASIA_RECORD="$OUT/restore/session.log" node web/run-node.mjs "$WASM" "$OUT/restore/base.eus" > "$OUT/restore/recorded.txt"
expected=$(printf '%s\n' 'before lazy 2 ordinary 4' 'after lazy 2 ordinary 4')
[ "$(cat "$OUT/restore/recorded.txt")" = "$expected" ] || { cat "$OUT/restore/recorded.txt"; echo "unexpected output before the restore"; exit 1; }
"$E" --checkpoint "$OUT/restore/session.log" 1 "$OUT/restore/checkpoint.eus"
node web/run-node.mjs "$WASM" "$OUT/restore/checkpoint.eus" > "$OUT/restore/restored.txt"
expected=$(printf '%s\n' 'after lazy 2 ordinary failed' 'restored lazy 2 ordinary failed')
[ "$(cat "$OUT/restore/restored.txt")" = "$expected" ] || { cat "$OUT/restore/restored.txt"; echo "unexpected output after the restore"; exit 1; }
cat "$OUT/restore/restored.txt"
echo "Replay: logs replay to the same outputs, from the start and from checkpoints, across wasm and native; checkpoints restore into a new host."
