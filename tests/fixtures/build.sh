#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

# ARM Cortex-M (thumb)
arm-none-eabi-gcc -O0 -g -mcpu=cortex-m4 -mthumb -nostartfiles \
  -ffunction-sections -fdata-sections \
  src/arm/main.c src/arm/util.c \
  -o arm.elf

echo "Built arm.elf ($(stat -f%z arm.elf 2>/dev/null || stat -c%s arm.elf) bytes)"
