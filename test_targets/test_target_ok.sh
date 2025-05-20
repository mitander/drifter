#!/bin/bash
# Reads from stdin and echoes to stdout, exits 0
cat
exit 0
```    *   `test_target_exit_code_crash.sh`:
```bash
#!/bin/bash
# Exits with the provided argument or 1 if no argument
exit "${1:-1}"
