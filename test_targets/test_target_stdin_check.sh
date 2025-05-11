#!/bin/sh
# Reads stdin, if it's "CRASHME", exits 1, else 0
read input
if [ "$input" = "CRASHME" ]; then
  exit 1
fi
exit 0
