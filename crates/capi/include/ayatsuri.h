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
 The ABI version this header describes. Bumped only on a breaking
 change to it; a host compares it with `ay2d_abi_version()`.
 */
#define AY2D_ABI_VERSION 1

/*
 Drawn with additive blending.
 */
#define AY2D_FLAG_ADDITIVE 1

/*
 Drawn with multiplicative blending.
 */
#define AY2D_FLAG_MULTIPLICATIVE 2

/*
 Both faces drawn; no back-face culling.
 */
#define AY2D_FLAG_DOUBLE_SIDED 4

/*
 The masks cut the mesh out where they are drawn, not where they are not.
 */
#define AY2D_FLAG_INVERTED_MASK 8

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
 One mesh to draw: where its vertices are now, where they read the
 texture, and how. `positions` and `uvs` hold `vertex_count` xy pairs in
 model units and 0 to 1; `indices` a triangle list into them; `masks`
 the meshes (by mesh index) whose texture alpha clips this one.
 */
typedef struct ay2d_drawable {
  uint32_t mesh;
  uint32_t texture;
  /*
   `AY2D_FLAG_*` bits.
   */
  uint32_t flags;
  float opacity;
  uint32_t vertex_count;
  uint32_t index_count;
  uint32_t mask_count;
  const float *positions;
  const float *uvs;
  const uint16_t *indices;
  const uint32_t *masks;
} ay2d_drawable;

/*
 The canvas the model is drawn on: pixels, and how model units map
 onto them (`px = unit * pixels_per_unit + origin`).
 */
typedef struct ay2d_canvas_info {
  float width;
  float height;
  float pixels_per_unit;
  float origin_x;
  float origin_y;
} ay2d_canvas_info;

/*
 Facts about a clip, for a host's motion list.
 */
typedef struct ay2d_motion_info {
  /*
   Seconds.
   */
  float duration;
  float fps;
  /*
   1 when the clip was authored to loop.
   */
  int32_t looped;
  /*
   1 when the clip is a composition rather than a scene.
   */
  int32_t composition;
  /*
   Authored fades in seconds; -1 when the clip leaves them to the player.
   */
  float fade_in;
  float fade_out;
  uint32_t event_count;
} ay2d_motion_info;

/*
 An event a motion fired since the last update.
 */
typedef struct ay2d_event {
  /*
   The motion that fired it, by index.
   */
  uint32_t motion;
  /*
   Its time on the motion's clock, in seconds.
   */
  float time;
  /*
   The event's text, NUL-terminated, alive as long as the model.
   */
  const char *name;
} ay2d_event;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/*
 The ABI version the library was built with, for a host to check
 against `AY2D_ABI_VERSION` before anything else.
 */
uint32_t ay2d_abi_version(void);

/*
 The library's own version, as a NUL-terminated string that lives as
 long as the library.
 */
const char *ay2d_version(void);

/*
 The visible meshes after the last update, back to front, into `out`.
 Returns how many there are; with `n` smaller than that (zero and a
 null `out` to just ask) only the first `n` are written. Masks are not
 among the drawables unless drawn in their own right; use
 `ay2d_mesh_geometry` to render a mask.

 `m` is a live handle; `out` holds `n` writable drawables, or `n` is zero.
 */
uint32_t ay2d_drawables(const struct ay2d_model *m, struct ay2d_drawable *out, size_t n);

/*
 A mesh's geometry right now, visible or not: for rendering a mask, or
 for a host that keeps everything on the GPU and only streams positions.
 `uvs` and `indices` never change for a mesh; upload them once and read
 `positions` after each update. Fails with range past the mesh count.

 `m` is a live handle; `out` points at one writable drawable.
 */
enum ay2d_status ay2d_mesh_geometry(const struct ay2d_model *m,
                                    uint32_t mesh,
                                    struct ay2d_drawable *out);

/*
 Why the last call on this thread failed, NUL-terminated; empty when it
 did not. The pointer is good until the next failing call on the thread.
 */
const char *ay2d_last_error(void);

/*
 How many expressions the model carries.

 `m` is a live handle.
 */
uint32_t ay2d_expression_count(const struct ay2d_model *m);

/*
 An expression's name, NUL-terminated, alive as long as the model.

 `m` is a live handle.
 */
const char *ay2d_expression_id(const struct ay2d_model *m, uint32_t i);

/*
 The index of the expression with this name, or -1.

 `m` is a live handle; `name` is a NUL-terminated string.
 */
int32_t ay2d_expression_index(const struct ay2d_model *m, const char *name);

/*
 Show expression `i`: the one shown before fades out over its own fade
 while this one fades in over `fade_in` seconds (below zero: its
 authored fade or the spec's second; zero: at once).

 `m` is a live handle.
 */
enum ay2d_status ay2d_expression_set(struct ay2d_model *m, uint32_t i, float fade_in);

/*
 Fade the shown expression out over `fade_out` seconds (below zero: its
 own fade or the spec's second; zero: at once).

 `m` is a live handle.
 */
enum ay2d_status ay2d_expression_clear(struct ay2d_model *m, float fade_out);

/*
 The expression shown (the last one set and not cleared), or -1.

 `m` is a live handle.
 */
int32_t ay2d_expression_shown(const struct ay2d_model *m);

/*
 Load a model from the bytes of an `.aom` file. The bytes (`len` of them
 at `bytes`) are read, not kept: the host may free them as soon as this
 returns. Null when the bytes are not a model; `ay2d_last_error` says
 why. The model is evaluated once at its defaults, so drawables are
 ready at once.
 */
struct ay2d_model *ay2d_load(const uint8_t *bytes, size_t len);

/*
 Free a model, once. Null is fine. Every pointer the model handed out
 (ids, drawables, textures) dies with it.
 */
void ay2d_free(struct ay2d_model *m);

/*
 How many parameters the model has.

 `m` is a live handle.
 */
uint32_t ay2d_parameter_count(const struct ay2d_model *m);

/*
 How many parts the model has.

 `m` is a live handle.
 */
uint32_t ay2d_part_count(const struct ay2d_model *m);

/*
 How many meshes the model has; a drawable's `mesh` is an index into
 them, and so is every mesh-wise call.

 `m` is a live handle.
 */
uint32_t ay2d_mesh_count(const struct ay2d_model *m);

/*
 How many texture pages the model carries.

 `m` is a live handle.
 */
uint32_t ay2d_texture_count(const struct ay2d_model *m);

/*
 The canvas: size in pixels and the model-unit to pixel mapping.

 `m` is a live handle; `out` points at one writable canvas.
 */
enum ay2d_status ay2d_canvas(const struct ay2d_model *m, struct ay2d_canvas_info *out);

/*
 A parameter's id (`ParamAngleX`...), NUL-terminated, alive as long as
 the model. Null past the count.

 `m` is a live handle.
 */
const char *ay2d_parameter_id(const struct ay2d_model *m, uint32_t i);

/*
 A part's id, as [`ay2d_parameter_id`].

 `m` is a live handle.
 */
const char *ay2d_part_id(const struct ay2d_model *m, uint32_t i);

/*
 A mesh's id, as [`ay2d_parameter_id`].

 `m` is a live handle.
 */
const char *ay2d_mesh_id(const struct ay2d_model *m, uint32_t i);

/*
 The index of the parameter with this id, or -1 when the model has none.
 Resolve ids once at load; drive by index after.

 `m` is a live handle; `id` is a NUL-terminated string.
 */
int32_t ay2d_parameter_index(const struct ay2d_model *m, const char *id);

/*
 How many motions the model carries: its scenes, then its compositions.

 `m` is a live handle.
 */
uint32_t ay2d_motion_count(const struct ay2d_model *m);

/*
 A motion's name, NUL-terminated, alive as long as the model. Null past
 the count.

 `m` is a live handle.
 */
const char *ay2d_motion_id(const struct ay2d_model *m, uint32_t i);

/*
 The index of the motion with this name, or -1.

 `m` is a live handle; `name` is a NUL-terminated string.
 */
int32_t ay2d_motion_index(const struct ay2d_model *m, const char *name);

/*
 Facts about motion `i`.

 `m` is a live handle; `out` points at one writable info.
 */
enum ay2d_status ay2d_motion_info(const struct ay2d_model *m,
                                  uint32_t i,
                                  struct ay2d_motion_info *out);

/*
 Start motion `i` from its beginning. Whatever was playing fades out
 over its own fade (or the spec's second) while this one fades in over
 `fade_in` seconds; `fade_in` below zero takes the motion's authored
 fade, zero starts it at full weight. `looping` repeats it; zero plays
 it once and holds its last frame.

 `m` is a live handle.
 */
enum ay2d_status ay2d_motion_play(struct ay2d_model *m, uint32_t i, int32_t looping, float fade_in);

/*
 Fade every playing motion out over `fade_out` seconds (below zero: the
 motion's own fade or the spec's second; zero: at once).

 `m` is a live handle.
 */
enum ay2d_status ay2d_motion_stop(struct ay2d_model *m, float fade_out);

/*
 The motion in front (the last one started and not stopped), or -1.

 `m` is a live handle.
 */
int32_t ay2d_motion_playing(const struct ay2d_model *m);

/*
 1 when the motion in front played once through and now holds its last
 frame; 0 while it runs, loops, or when nothing plays.

 `m` is a live handle.
 */
int32_t ay2d_motion_finished(const struct ay2d_model *m);

/*
 Where the motion in front is on its clock, in seconds; -1 when nothing
 plays.

 `m` is a live handle.
 */
float ay2d_motion_time(const struct ay2d_model *m);

/*
 Put the motion in front at `t` seconds on its clock.

 `m` is a live handle.
 */
enum ay2d_status ay2d_motion_seek(struct ay2d_model *m, float t);

/*
 Play the motion in front at `speed` times its rate (1 is as authored).

 `m` is a live handle.
 */
enum ay2d_status ay2d_motion_set_speed(struct ay2d_model *m, float speed);

/*
 The events motions fired during the last `ay2d_update`, into `out`.
 Returns how many there were; with `n` smaller only the first `n` are
 written. Poll after each update; the list is replaced by the next one.

 `m` is a live handle; `out` holds `n` writable events, or `n` is zero.
 */
uint32_t ay2d_motion_events(const struct ay2d_model *m, struct ay2d_event *out, size_t n);

/*
 Every parameter's minimum, maximum and default, by index. Each array
 holds `ay2d_parameter_count` floats; any may be null to skip it.

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

 `m` is a live handle; `values` holds `n` writable floats.
 */
enum ay2d_status ay2d_get_parameters(const struct ay2d_model *m, float *values, size_t n);

/*
 Set `n` parameters at once: `indices[i]` takes `values[i]`. Values are
 clamped to the parameter's range. An index past the model fails the
 whole call before anything changes. Nothing is evaluated until
 `ay2d_update`.

 `m` is a live handle; `indices` and `values` hold `n` readable items.
 */
enum ay2d_status ay2d_set_parameters(struct ay2d_model *m,
                                     const uint32_t *indices,
                                     const float *values,
                                     size_t n);

/*
 Set every parameter from one array of `ay2d_parameter_count` values,
 by index. The usual frame call for a host that keeps the whole pose.

 `m` is a live handle; `values` holds `n` readable floats.
 */
enum ay2d_status ay2d_set_all_parameters(struct ay2d_model *m, const float *values, size_t n);

/*
 Every part's host opacity, by index, into `values`
 (`ay2d_part_count` floats, 0 to 1; 1 when never set).

 `m` is a live handle; `values` holds `n` writable floats.
 */
enum ay2d_status ay2d_get_part_opacities(const struct ay2d_model *m, float *values, size_t n);

/*
 Set `n` parts' host opacities at once: `indices[i]` takes `values[i]`,
 clamped to 0 to 1. An index past the model fails the whole call before
 anything changes. Nothing is evaluated until `ay2d_update`.

 `m` is a live handle; `indices` and `values` hold `n` readable items.
 */
enum ay2d_status ay2d_set_part_opacities(struct ay2d_model *m,
                                         const uint32_t *indices,
                                         const float *values,
                                         size_t n);

/*
 Advance the model by `dt` seconds and evaluate it: playing motions and
 the shown expression write their parameters over the pose, physics
 steps (when the model has a rig and it is on), parameters resolve,
 deformers and meshes move, draw order sorts. After this
 `ay2d_drawables` and `ay2d_motion_events` are current. `dt` of zero
 evaluates without moving motions or physics.

 `m` is a live handle.
 */
enum ay2d_status ay2d_update(struct ay2d_model *m, float dt);

/*
 1 when the model carries a pendulum rig, else 0.

 `m` is a live handle.
 */
int32_t ay2d_has_physics(const struct ay2d_model *m);

/*
 Run (1) or hold (0) the rig on update. On by default when the model
 has one; off, the physics-driven parameters keep their set values.

 `m` is a live handle.
 */
enum ay2d_status ay2d_set_physics(struct ay2d_model *m, int32_t on);

/*
 Wind on the rig, in the same units gravity has (down is 0, -1).

 `m` is a live handle.
 */
enum ay2d_status ay2d_set_wind(struct ay2d_model *m, float x, float y);

/*
 Put every pendulum back at rest, as after a scene cut.

 `m` is a live handle.
 */
enum ay2d_status ay2d_reset_physics(struct ay2d_model *m);

/*
 Texture page `i` as encoded bytes (PNG), alive as long as the model.
 A drawable's `texture` is an index into these.

 `m` is a live handle; `bytes` and `len` each point at one writable slot.
 */
enum ay2d_status ay2d_texture(const struct ay2d_model *m,
                              uint32_t i,
                              const uint8_t **bytes,
                              size_t *len);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* AYATSURI_H */
