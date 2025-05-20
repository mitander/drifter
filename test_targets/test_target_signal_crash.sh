#!/bin/bash
# Crashes with a signal (e.g., SIGSEGV or SIGABRT)
# This example uses an undefined variable, which often leads to a crash by the shell,
# or you can use a command known to cause a specific signal.
if [[ "$OSTYPE" == "darwin"* || "$OSTYPE" == "linux-gnu"* ]]; then
  kill -SEGV $$ # Send SIGSEGV to self
else
  # Fallback for other systems, might not produce the desired signal
  echo "Cannot reliably send SIGSEGV on this OS for test" >&2
  exit 123 # Some distinct error code
fi
