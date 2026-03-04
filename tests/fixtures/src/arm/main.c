#include <stdint.h>

extern int32_t util_add(int32_t a, int32_t b);
extern int32_t util_mul(int32_t a, int32_t b);

const char version[] __attribute__((used)) = "1.0.0";
const uint8_t lookup_table[256] __attribute__((used)) = {0};

int32_t app_init(void) {
    return util_add(1, 2);
}

int32_t app_run(void) {
    int32_t x = util_mul(3, 4);
    return util_add(x, 5);
}

void _start(void) {
    app_init();
    app_run();
}
