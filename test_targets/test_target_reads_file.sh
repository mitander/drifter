#!/bin/bash
# Reads content from the file path provided as $1
# Exits 0 if content is "EXPECTED_CONTENT_OK"
# Exits 1 otherwise
if [ -z "$1" ]; then
  echo "Error: No file path provided." >&2
  exit 2
fi
if [ ! -f "$1" ]; then
  echo "Error: File '$1' not found." >&2
  exit 3
fi

content=$(cat "$1")
if [ "$content" == "EXPECTED_CONTENT_OK" ]; then
  exit 0
else
  exit 1
fi
