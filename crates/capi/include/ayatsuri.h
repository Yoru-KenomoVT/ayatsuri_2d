/*
 * ayatsuri.h: the Ayatsuri2D runtime for anything that can call C.
 *
 * Coarse by design: arrays in, arrays out, never a call per parameter.
 * A model is an opaque handle; every call on it is cheap and none allocate
 * per frame. Panics never cross this boundary: a failing call returns a
 * status or a null and ay2d_last_error() says why.
 *
 * Generated from crates/capi by cbindgen; edit the Rust, not this file.
 * Apache-2.0. See LICENSE beside this header.
 */

#ifndef AYATSURI_H
#define AYATSURI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/*
 The ABI version a host checks before anything else. Bumped only on a
 breaking change to the header.
 */
uint32_t ay2d_abi_version(void);

/*
 The library's own version, as a NUL-terminated string that lives as
 long as the library.
 */
const char *ay2d_version(void);

/*
 Why the last call on this thread failed, NUL-terminated; empty when it
 did not. The pointer is good until the next failing call on the thread.
 */
const char *ay2d_last_error(void);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* AYATSURI_H */
