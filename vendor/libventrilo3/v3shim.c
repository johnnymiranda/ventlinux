#include "ventrilo3.h"
#include "v3shim.h"

const char *v3_last_error(void) {
    return _v3_error(NULL);
}
