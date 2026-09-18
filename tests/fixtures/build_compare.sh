#!/bin/bash
# Fixtures for comparison-mode tests (tests/compare.rs). Each pair differs in a
# way that stresses symbol identity or group membership; see the test comments.
set -euo pipefail
cd "$(dirname "$0")/src/compare"
CC="arm-none-eabi-gcc -O0 -g -mcpu=cortex-m4 -mthumb -nostartfiles -ffunction-sections -fdata-sections"

# Per-ELF common path prefix changes when a file outside sub/ is added.
(cd prefix_before && $CC sub/a.c sub/b.c -o ../../../prefix_before.elf 2>/dev/null)
(cd prefix_after  && $CC sub/a.c sub/b.c c.c -o ../../../prefix_after.elf 2>/dev/null)

# static helper() in shared.h, included by a.c and b.c; only a.c's copy grows.
(cd shared_header_before && $CC m.c a.c b.c -o ../../../shared_header_before.elf)
(cd shared_header_after  && $CC m.c a.c b.c -o ../../../shared_header_after.elf)

# src/ gains a whole new file (src/c.c) in the second ELF.
(cd group_before && $CC m.c src/a.c src/b.c -o ../../../group_before.elf)
(cd group_after  && $CC m.c src/a.c src/b.c src/c.c -o ../../../group_after.elf)

# Same relative dirs, but old/b.c is replaced by a *different* new/b.c (each
# with its own local helper): a shared src/a.c anchor exists, and b.c must not
# be treated as "the same file" just because the basenames match.
(cd dir_move_before && $CC src/a.c old/b.c -o ../../../dir_move_before.elf 2>/dev/null)
(cd dir_move_after  && $CC src/a.c new/b.c -o ../../../dir_move_after.elf 2>/dev/null)

# Exactly one static helper() per ELF, from different translation units
# (a.c vs b.c) that include the same shared.h: cardinality is 1 on each side.
(cd tu_before && $CC a.c -o ../../../tu_before.elf 2>/dev/null)
(cd tu_after  && $CC b.c -o ../../../tu_after.elf 2>/dev/null)
