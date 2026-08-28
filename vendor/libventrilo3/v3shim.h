/*
 * Small C shims for things Swift can't call directly (variadic functions).
 */
#ifndef _V3SHIM_H
#define _V3SHIM_H

/* Returns the last error string from libventrilo3 (wraps variadic _v3_error). */
const char *v3_last_error(void);

#endif
