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

/*
 What a call reports. Zero is success, as C expects.
 */
typedef enum ay2d_status {
  AY2D_STATUS_OK = 0,
  /*
   A required pointer was null.
   */
  AY2D_STATUS_NULL = 1,
  /*
   The bytes were not a model this library reads.
   */
  AY2D_STATUS_BAD_MODEL = 2,
  /*
   An index pointed past the model.
   */
  AY2D_STATUS_RANGE = 3,
  /*
   An output array was shorter than what it has to hold.
   */
  AY2D_STATUS_TOO_SMALL = 4,
  /*
   Something inside went wrong; `ay2d_last_error` says what.
   */
  AY2D_STATUS_INTERNAL = 5,
} ay2d_status;

/*
 A loaded model: the puppet and what the ABI keeps beside it.
 */
typedef struct ay2d_model ay2d_model;

/*
 The canvas the model is drawn on: pixels, and how model units map
 onto them (`px = unit * pixels_per_unit + origin`).
 */
typedef struct ay2d_canvas {
  float width;
  float height;
  float pixels_per_unit;
  float origin_x;
  float origin_y;
} ay2d_canvas;

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

/*
 Load a model from the bytes of an `.aom` file. The bytes are read, not
 kept: the host may free them as soon as this returns. Null when the
 bytes are not a model; `ay2d_last_error` says why. The model is
 evaluated once at its defaults, so drawables are ready at once.

 # Safety
 `bytes` points at `len` readable bytes.
 */
struct ay2d_model *ay2d_load(const uint8_t *bytes, size_t len);

/*
 Free a model. Null is fine. Every pointer the model handed out (ids,
 drawables, textures) dies with it.

 # Safety
 `m` is null or a handle `ay2d_load` returned, freed once.
 */
void ay2d_free(struct ay2d_model *m);

/*
 How many parameters the model has.

 # Safety
 `m` is a live handle.
 */
uint32_t ay2d_parameter_count(const struct ay2d_model *m);

/*
 How many parts the model has.

 # Safety
 `m` is a live handle.
 */
uint32_t ay2d_part_count(const struct ay2d_model *m);

/*
 How many meshes the model has; a drawable's `mesh` is an index into
 them, and so is every mesh-wise call.

 # Safety
 `m` is a live handle.
 */
uint32_t ay2d_mesh_count(const struct ay2d_model *m);

/*
 How many texture pages the model carries.

 # Safety
 `m` is a live handle.
 */
uint32_t ay2d_texture_count(const struct ay2d_model *m);

/*
 The canvas: size in pixels and the model-unit to pixel mapping.

 # Safety
 `m` is a live handle; `out` points at one writable canvas.
 */
enum ay2d_status ay2d_canvas(const struct ay2d_model *m, struct ay2d_canvas *out);

/*
 A parameter's id (`ParamAngleX`...), NUL-terminated, alive as long as
 the model. Null past the count.

 # Safety
 `m` is a live handle.
 */
const char *ay2d_parameter_id(const struct ay2d_model *m, uint32_t i);

/*
 A part's id, as [`ay2d_parameter_id`].

 # Safety
 `m` is a live handle.
 */
const char *ay2d_part_id(const struct ay2d_model *m, uint32_t i);

/*
 A mesh's id, as [`ay2d_parameter_id`].

 # Safety
 `m` is a live handle.
 */
const char *ay2d_mesh_id(const struct ay2d_model *m, uint32_t i);

/*
 The index of the parameter with this id, or -1 when the model has none.
 Resolve ids once at load; drive by index after.

 # Safety
 `m` is a live handle; `id` is a NUL-terminated string.
 */
int32_t ay2d_parameter_index(const struct ay2d_model *m, const char *id);

/*
 Every parameter's minimum, maximum and default, by index. Each array
 holds `ay2d_parameter_count` floats; any may be null to skip it.

 # Safety
 `m` is a live handle; each non-null array holds `n` writable floats.
 */
enum ay2d_status ay2d_parameter_ranges(const struct ay2d_model *m,
                                       float *min,
                                       float *max,
                                       float *default_,
                                       size_t n);

/*
 Every parameter's current value, by index, into `values`
 (`ay2d_parameter_count` floats).

 # Safety
 `m` is a live handle; `values` holds `n` writable floats.
 */
enum ay2d_status ay2d_get_parameters(const struct ay2d_model *m, float *values, size_t n);

/*
 Set `n` parameters at once: `indices[i]` takes `values[i]`. Values are
 clamped to the parameter's range. An index past the model fails the
 whole call before anything changes. Nothing is evaluated until
 `ay2d_update`.

 # Safety
 `m` is a live handle; `indices` and `values` hold `n` readable items.
 */
enum ay2d_status ay2d_set_parameters(struct ay2d_model *m,
                                     const uint32_t *indices,
                                     const float *values,
                                     size_t n);

/*
 Set every parameter from one array of `ay2d_parameter_count` values,
 by index. The usual frame call for a host that keeps the whole pose.

 # Safety
 `m` is a live handle; `values` holds `n` readable floats.
 */
enum ay2d_status ay2d_set_all_parameters(struct ay2d_model *m, const float *values, size_t n);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* AYATSURI_H */
