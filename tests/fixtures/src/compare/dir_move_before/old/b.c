#include <stdint.h>
static int32_t helper(int32_t x) { return x + 1; }
int32_t use_b(int32_t x) { return helper(x); }
