#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Reflection (M6, newspeak/EURASIA_M6_REFLECTION_DESIGN_2026-11-18.md): the
# compiler probe compiles a class from source on Eurasia with psoup's
# compiler, builds it into program objects (Method new, InstanceMixin new,
# ClassMixin new), applies them and runs the class; its output is checked,
# also under GC stress. The debugger probe checks that a broken thread, made
# by the handler of last resort as the IDE makes it, can be inspected after
# its turn has ended. Needs what scripts/genesis.sh needs. Outputs go to
# target/reflection/.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
PSOUP=$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup
P=$PRIMORDIALSOUP/newspeak
OUT=$EURASIA/target/reflection
mkdir -p "$OUT"

echo "== building EurasiaPackager.vfuel"
(cd "$PRIMORDIALSOUP" && "$PSOUP" ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  ./newspeak/platform/PrimordialSoup/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  PrimordialSoup/RuntimeWithMirrors EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }

echo "== the compiler probe"
MODULES="$P/Kernel.ns $P/platform/PrimordialSoup/Collections.ns $NEWSPEAK/platform/Eurasia/Actors.ns $NEWSPEAK/platform/Eurasia/Runtime.ns $P/NewspeakASTs.ns $P/NewspeakPredictiveParsing.ns $P/IntermediatesForPrimordialSoup.ns $P/NewspeakCompilation.ns $P/NewspeakBytecodeCompilation.ns $NEWSPEAK/Newspeak2V5BytecodeCompilation.ns"
# shellcheck disable=SC2086
"$PSOUP" "$OUT/EurasiaPackager.vfuel" --entry-in-kernel "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv \
  "$NEWSPEAK/EurasiaCompilerProbe.ns" $MODULES "$OUT/genesis.eus" > "$OUT/package.log"
cargo build -q --release -p eurasia-native
EURASIA_BIN=$EURASIA/target/release/eurasia
(cd "$OUT" && "$EURASIA_BIN" genesis.eus base.eus)
"$EURASIA_BIN" "$OUT/base.eus" | tee "$OUT/output.txt"
expected=$(printf '%s\n' 'compiled Probe' 42 56 'built at run time!' 'class side' done)
[ "$(cat "$OUT/output.txt")" = "$expected" ] || { echo "Unexpected output from the compiler probe."; exit 1; }
EURASIA_GC_STRESS=200 "$EURASIA_BIN" "$OUT/base.eus" > "$OUT/output-stress.txt"
cmp "$OUT/output.txt" "$OUT/output-stress.txt" || { echo "GC stress changed the output"; exit 1; }
echo "== the debugger probe: a broken thread outlives its turn"
REFLECTIVE="$P/Kernel.ns $P/platform/PrimordialSoup/Collections.ns $NEWSPEAK/platform/Eurasia/Actors.ns $NEWSPEAK/platform/Eurasia/Runtime.ns $NEWSPEAK/platform/Eurasia/Mirrors.ns $NEWSPEAK/platform/Eurasia/RuntimeWithMirrors.ns $P/NewspeakASTs.ns $P/NewspeakPredictiveParsing.ns $P/IntermediatesForPrimordialSoup.ns $P/NewspeakCompilation.ns $P/NewspeakBytecodeCompilation.ns $NEWSPEAK/Newspeak2V5BytecodeCompilation.ns $P/MetadataParsing.ns"
mkdir -p "$OUT/debugger"
# shellcheck disable=SC2086
"$PSOUP" "$OUT/EurasiaPackager.vfuel" --entry-in-kernel "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv \
  "$NEWSPEAK/EurasiaDebuggerProbe.ns" $REFLECTIVE "$OUT/debugger/genesis.eus" > "$OUT/debugger/package.log"
(cd "$OUT/debugger" && "$EURASIA_BIN" genesis.eus base.eus)
"$EURASIA_BIN" "$OUT/debugger/base.eus" | tee "$OUT/debugger/output.txt"
expected=$(printf '%s\n' \
  "Exception in turn without resolver: instance of EurasiaDebuggerProbe 'main:args:' -- MessageNotUnderstood: SmallInteger foo" \
  'broken true' \
  'stack doesNotUnderstand: failingMethod: main:args: object:perform:with: sendTo: deliver on:do: deliver drainQueue drainQueue dispatchMessage:port:' \
  'receiver true' done)
[ "$(cat "$OUT/debugger/output.txt")" = "$expected" ] || { echo "Unexpected output from the debugger probe."; exit 1; }
EURASIA_GC_STRESS=200 "$EURASIA_BIN" "$OUT/debugger/base.eus" > "$OUT/debugger/output-stress.txt"
cmp "$OUT/debugger/output.txt" "$OUT/debugger/output-stress.txt" || { echo "GC stress changed the debugger probe's output"; exit 1; }
echo "== deploying from what is running: an application writes a genesis image of itself"
# The build and deploy design, section 4.2: platform mirrors
# genesisImageOfApplicationKeepingSources: (the native VmGenesisImage). The
# image it writes must hold the same methods as the one packaged from source
# (eurasia --methods), and run, afresh, as that one does.
DEPLOY=$OUT/deploy
python3 scripts/tool-image.py "$NEWSPEAK/EurasiaDeployProbe.ns" "$DEPLOY" > /dev/null
"$EURASIA_BIN" "$DEPLOY/base.eus" deploy "$DEPLOY/self.genesis.eus" | tee "$DEPLOY/deploying.txt"
grep -q '^deployed [0-9]* bytes$' "$DEPLOY/deploying.txt" || { echo "the probe did not deploy itself"; exit 1; }
"$EURASIA_BIN" --methods "$DEPLOY/genesis.eus" > "$DEPLOY/packaged-methods.txt"
"$EURASIA_BIN" --methods "$DEPLOY/self.genesis.eus" > "$DEPLOY/deployed-methods.txt"
cmp "$DEPLOY/packaged-methods.txt" "$DEPLOY/deployed-methods.txt" \
  || { echo "the deployed image's methods differ from the packaged image's"; exit 1; }
(cd "$DEPLOY" && "$EURASIA_BIN" self.genesis.eus self.base.eus)
[ "$("$EURASIA_BIN" "$DEPLOY/self.base.eus")" = "running, run 1" ] \
  || { echo "the deployed image does not run afresh"; exit 1; }
echo "deployed: $(wc -l < "$DEPLOY/deployed-methods.txt") methods, as packaged; the image runs afresh"
echo "Reflection: the compiler runs on Eurasia and its classes run; a broken thread outlives its turn; an application deploys itself."
