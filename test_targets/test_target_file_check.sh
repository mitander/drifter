#!/bin/sh
# Reads from file, if it's "CRASHFILE", exits 1, else 0
input=$(cat "$1")
if [ "$input" = "CRASHFILE" ]; then
  exit 1
fi
exit 0
