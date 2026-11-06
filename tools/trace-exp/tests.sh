. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
date
cargo test --release 2>&1 | grep -E '^test result|FAILED|panicked|failed|error' | head -40
echo "cargo test exit ${PIPESTATUS[0]}"
for c in examples/*/check.sh; do
  date
  bash $c > $S/check.$(basename $(dirname $c)).log 2>&1
  echo "$c exit $?"
done
date
echo TESTS DONE
