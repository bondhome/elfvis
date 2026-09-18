#include <stdint.h>
#ifdef GROW
static int32_t helper(int32_t x) { return x + 1 + x*2 + x*3 + x*5 + x*7 + x*11; }
#else
static int32_t helper(int32_t x) { return x + 1; }
#endif
