#include <stdint.h>
static int32_t helper(int32_t x) { return x + 1 + x*2 + x*3 + x*5 + x*7 + x*11; }
int32_t use_b(int32_t x) { return helper(x); }
