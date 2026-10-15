# Ayatsuri2D C ABI reference

The runtime for anything that calls C. This is the developer documentation
for `ayatsuri.h`, version 1 of the ABI. The header itself is generated from
the Rust source and carries the same text per function; this document adds
the model behind it: lifecycle, ownership, threading, rendering, and how to
bind it from other languages.

Contents

1. [Model](#1-model)
2. [Lifecycle](#2-lifecycle)
3. [Errors](#3-errors)
4. [Ownership and lifetimes](#4-ownership-and-lifetimes)
5. [Threading](#5-threading)
6. [Functions](#6-functions)
7. [Types and constants](#7-types-and-constants)
8. [Rendering guide](#8-rendering-guide)
9. [Physics](#9-physics)
10. [Performance](#10-performance)
11. [Binding from other languages](#11-binding-from-other-languages)
12. [Versioning and stability](#12-versioning-and-stability)
13. [Building and linking](#13-building-and-linking)

## 1. Model

A model is a rig: parameters a host drives, parts that group things and can
fade, meshes that are drawn, deformers that move them, and a physics rig
that swings some parameters on its own. The host never sees deformers or
keyforms; it sets parameters, calls update, and reads where every mesh's
vertices ended up.

    host                         ay2d_model
    ───────────────              ─────────────────────────────────────
    parameters (floats) ──────►  resolve ─► deformers ─► meshes ─► sort
    part opacities      ──────►                              │
    dt (physics)        ──────►  rig steps first             ▼
                        ◄──────  drawables: positions, uvs, triangles,
                                 masks, flags, opacity, back to front

Coordinates: mesh positions are model units. `ay2d_canvas` gives the
mapping to the authored canvas, `px = unit * pixels_per_unit + origin`, with
pixel y running down as in the texture. UVs are 0 to 1 over the texture
page, `v = 0` at the top row of the image.

Models are `.aom` files, Ayatsuri's own container, written by the editor.
The ABI reads nothing else on purpose: the moc3 parser is hostile-input
code the runtime never compiles ([integration.md](../../../docs/integration.md)).

## 2. Lifecycle

```c
#include "ayatsuri.h"

if (ay2d_abi_version() != AY2D_ABI_VERSION) { /* header and library disagree */ }

ay2d_model *m = ay2d_load(bytes, len);     /* bytes may be freed right after */
if (!m) { puts(ay2d_last_error()); return; }

/* once: resolve the ids you care about to indices */
int32_t ax = ay2d_parameter_index(m, "ParamAngleX");
int32_t ay = ay2d_parameter_index(m, "ParamAngleY");

/* every frame */
uint32_t idx[2] = { ax, ay };
float    val[2] = { head_yaw, head_pitch };
ay2d_set_parameters(m, idx, val, 2);
ay2d_update(m, dt);
uint32_t n = ay2d_drawables(m, out, cap);   /* then draw out[0..n) in order */

ay2d_free(m);
```

`ay2d_load` validates the whole file (every index checked, every array
sized) and evaluates the model once at its defaults, so drawables are ready
before the first update. Load is the only call that allocates in
proportion to the model; the frame calls allocate nothing.

## 3. Errors

Every function returns an `ay2d_status`, a count, or a pointer.

| Returns | On failure |
|---|---|
| `ay2d_status` | a code other than `AY2D_STATUS_OK` |
| a count | `0` |
| a pointer | `NULL` |

A failing call changes nothing: `ay2d_set_parameters` with one bad index
sets none of them. `ay2d_last_error()` returns why the last call on the
current thread failed, as a NUL-terminated string that stays valid until
the next failing call on that thread. It is empty after a success only if
nothing failed before; check the status, not the string.

Panics inside the library are caught at the boundary and reported as
`AY2D_STATUS_INTERNAL` with the message in `ay2d_last_error()`. The host
process never aborts because of the runtime.

| Status | Meaning |
|---|---|
| `AY2D_STATUS_OK` | done |
| `AY2D_STATUS_NULL` | a required pointer was null (handle, array with a non-zero length, output slot) |
| `AY2D_STATUS_BAD_MODEL` | the bytes were not a model this library reads |
| `AY2D_STATUS_RANGE` | an index pointed past the model, or a value such as `dt` or wind was not finite |
| `AY2D_STATUS_TOO_SMALL` | an output array was shorter than what it has to hold |
| `AY2D_STATUS_INTERNAL` | a fault inside; see `ay2d_last_error()` and please report it |

## 4. Ownership and lifetimes

- The model owns everything it hands out. Nothing returned by the ABI is
  freed by the host, and `ay2d_free` releases it all.
- Ids (`ay2d_parameter_id`, `ay2d_part_id`, `ay2d_mesh_id`) live as long
  as the model. Read them once at load.
- `ay2d_version()` and `ay2d_last_error()` strings live as long as the
  library (the latter until the next failure on the thread).
- Drawable views (`positions`, `uvs`, `indices`, `masks` from
  `ay2d_drawables` and `ay2d_mesh_geometry`) point into the model's own
  buffers. They are valid until the next `ay2d_update` or `ay2d_free`.
  `uvs` and `indices` never change for a mesh; `positions` change every
  update.
- Texture bytes from `ay2d_texture` live as long as the model.
- The byte buffer given to `ay2d_load` is read and not kept.
- Arrays the host passes in are only read during the call.

## 5. Threading

A model is not thread-safe: one thread at a time per model, with no call
on it overlapping another. Different models may be used from different
threads at once. `ay2d_last_error()` is per thread. The library keeps no
global state besides that, so loading on a worker thread and driving on
the render thread is fine as long as the hand-over is sequenced.

## 6. Functions

Signatures are as in `ayatsuri.h`; `size_t n` is always the length of the
arrays beside it.

### Library

`uint32_t ay2d_abi_version(void)`
The ABI version the library was built with. Compare with
`AY2D_ABI_VERSION` from the header before anything else.

`const char *ay2d_version(void)`
The library version, e.g. `"0.5.6"`.

`const char *ay2d_last_error(void)`
See [Errors](#3-errors).

### Model

`ay2d_model *ay2d_load(const uint8_t *bytes, size_t len)`
Load an `.aom`. Validates, evaluates once at defaults. `NULL` with a
reason when the bytes are not a model. Physics is on after load when the
model has a rig.

`void ay2d_free(ay2d_model *m)`
Free a model. `NULL` is fine.

`uint32_t ay2d_parameter_count / ay2d_part_count / ay2d_mesh_count / ay2d_texture_count(const ay2d_model *m)`
Sizes of the four index spaces. A drawable's `mesh` and `texture`, and
every index in the array calls, live in these.

`ay2d_status ay2d_canvas(const ay2d_model *m, ay2d_canvas_info *out)`
Canvas size in pixels and the unit mapping.

`const char *ay2d_parameter_id / ay2d_part_id / ay2d_mesh_id(const ay2d_model *m, uint32_t i)`
The id of the object at index `i` (`ParamAngleX`, `PartArmL`, ...).
`NULL` past the count. These are the ids the model was rigged with and
the ones motion and expression files refer to.

`int32_t ay2d_parameter_index(const ay2d_model *m, const char *id)`
The index of a parameter by id, or `-1`. Resolve once; drive by index.

### Parameters

`ay2d_status ay2d_parameter_ranges(const ay2d_model *m, float *min, float *max, float *def, size_t n)`
Fill `min`, `max` and `def` with every parameter's range and default, by
index. Any of the three may be `NULL` to skip it. `n` must be at least
the parameter count.

`ay2d_status ay2d_get_parameters(const ay2d_model *m, float *values, size_t n)`
Every parameter's current value, by index. After an update with physics
on, physics-driven parameters show what the rig set.

`ay2d_status ay2d_set_parameters(ay2d_model *m, const uint32_t *indices, const float *values, size_t n)`
Set `n` parameters: `indices[i]` takes `values[i]`. Values are held to
the parameter's range; a NaN lands on the default. Any index past the
count fails the call with `RANGE` before anything changes. Nothing is
evaluated until `ay2d_update`.

`ay2d_status ay2d_set_all_parameters(ay2d_model *m, const float *values, size_t n)`
Set every parameter from one array of at least `parameter_count` values,
by index. The usual frame call for a host that keeps the whole pose.

### Parts

`ay2d_status ay2d_get_part_opacities(const ay2d_model *m, float *values, size_t n)`
Every part's host opacity (0 to 1; 1 when never set), by index.

`ay2d_status ay2d_set_part_opacities(ay2d_model *m, const uint32_t *indices, const float *values, size_t n)`
Set `n` parts' host opacities, clamped to 0 to 1. The host's fade
multiplies over whatever the model's own keyforms give the part, and a
part at 0 hides every mesh under it. Not evaluated until `ay2d_update`.

### Frame

`ay2d_status ay2d_update(ay2d_model *m, float dt)`
Advance by `dt` seconds and evaluate: the physics rig steps (when the
model has one and it is on), parameters resolve, deformers and meshes
move, the draw order sorts. `dt` of `0` evaluates without moving physics;
negative or non-finite `dt` is `RANGE`. After this call the drawables
are current. Call once per frame, after every set call of that frame.

### Physics

`int ay2d_has_physics(const ay2d_model *m)`
`1` when the model carries a rig.

`ay2d_status ay2d_set_physics(ay2d_model *m, int on)`
Run or hold the rig on update. Held, the physics-driven parameters keep
whatever the host sets.

`ay2d_status ay2d_set_wind(ay2d_model *m, float x, float y)`
A constant force on every pendulum, in the units gravity has (gravity is
`0, -1`). Non-finite is `RANGE`.

`ay2d_status ay2d_reset_physics(ay2d_model *m)`
Every pendulum back to rest, as after a scene cut or a teleport.

### Drawing

`uint32_t ay2d_drawables(const ay2d_model *m, ay2d_drawable *out, size_t n)`
Returns how many meshes are visible after the last update and writes the
first `min(n, count)` of them into `out`, back to front. Call with
`(NULL, 0)` to get the count, then with an array that size. See
[Rendering](#8-rendering-guide).

`ay2d_status ay2d_mesh_geometry(const ay2d_model *m, uint32_t mesh, ay2d_drawable *out)`
Any mesh's current geometry, visible or not, with `opacity` set to 1 and
the mesh's own flags and masks. For rendering a mask mesh that is not
itself drawn, and for a host that uploads `uvs` and `indices` once and
streams `positions` per frame.

`ay2d_status ay2d_texture(const ay2d_model *m, uint32_t i, const uint8_t **bytes, size_t *len)`
Texture page `i` as the file stores it: PNG bytes, straight (not
premultiplied) alpha. Decode with the host's image loader once at load.

## 7. Types and constants

```c
#define AY2D_ABI_VERSION 1u

typedef struct ay2d_model ay2d_model;           /* opaque */

typedef struct ay2d_canvas_info {
    float width, height;          /* pixels */
    float pixels_per_unit;
    float origin_x, origin_y;     /* pixel of model-unit (0, 0) */
} ay2d_canvas_info;

typedef struct ay2d_drawable {
    uint32_t mesh;                /* index into ay2d_mesh_count */
    uint32_t texture;             /* index into ay2d_texture_count */
    uint32_t flags;               /* AY2D_FLAG_* */
    float    opacity;             /* 0 to 1, multiply the mesh alpha */
    uint32_t vertex_count;
    uint32_t index_count;         /* a multiple of 3 */
    uint32_t mask_count;
    const float    *positions;    /* vertex_count xy pairs, model units */
    const float    *uvs;          /* vertex_count uv pairs, 0 to 1 */
    const uint16_t *indices;      /* triangle list */
    const uint32_t *masks;        /* mesh indices whose alpha clips this one */
} ay2d_drawable;

#define AY2D_FLAG_ADDITIVE        1u   /* add blend */
#define AY2D_FLAG_MULTIPLICATIVE  2u   /* multiply blend */
#define AY2D_FLAG_DOUBLE_SIDED    4u   /* draw both faces; no culling */
#define AY2D_FLAG_INVERTED_MASK   8u   /* masks cut where they are drawn */
```

Blend flags are exclusive: neither set means normal (source-over) blending.

## 8. Rendering guide

The ABI gives geometry; the host draws. One frame:

1. `ay2d_update(m, dt)`.
2. `n = ay2d_drawables(m, out, cap)`; draw `out[0]` first, `out[n-1]` last.
3. For each drawable, bind texture `out[i].texture`, upload or map
   `positions` (and `uvs`, `indices` once per mesh), set the blend from
   `flags`, modulate alpha by `opacity`, draw `index_count / 3` triangles.

### Blending

Textures are straight-alpha PNGs. With `color` and `alpha` sampled from the
texture and `a = alpha * opacity`:

| flags | result |
|---|---|
| none | `dst = color * a + dst * (1 - a)` |
| `ADDITIVE` | `dst = dst + color * a` |
| `MULTIPLICATIVE` | `dst = dst * (1 - a) + dst * color * a` |

A renderer that works in premultiplied alpha premultiplies at texture load
and uses `one, one-minus-src-alpha` for normal, `one, one` for additive.

### Masks

A drawable with `mask_count > 0` is clipped to where its mask meshes have
texture alpha. Mask meshes are often invisible themselves, so get their
geometry with `ay2d_mesh_geometry(m, masks[k], &g)`; their `positions` are
current after the same update.

Per masked drawable: render the mask meshes' alpha into a stencil or a
small offscreen (any blend, alpha only), then draw the mesh sampling that
coverage. With `AY2D_FLAG_INVERTED_MASK` draw where coverage is zero
instead. Group drawables by identical `masks` lists to render each mask
set once per frame.

### Culling

`AY2D_FLAG_DOUBLE_SIDED` means draw both faces. Without it a renderer may
cull back faces; triangle winding is as authored and a mesh can flip
through a deformer, so most hosts simply disable culling.

### Canvas and camera

Positions are model units; the authored picture is the rectangle from
unit `(-origin_x / ppu, -origin_y / ppu)` spanning `width / ppu` by
`height / ppu` (y down). Fit that rectangle to the viewport and the model
is framed as in the editor.

## 9. Physics

The rig is a set of pendulum chains driven by input parameters (head
angle, body angle...) that write output parameters (hair, accessories).
After `ay2d_load` it is on. `ay2d_update` steps it by `dt` before
evaluating; large `dt` (a hitch) is stepped as given, so clamp `dt` to a
sane maximum (say 1/15 s) if frames can stall. `ay2d_set_wind` adds a
constant force; `ay2d_reset_physics` settles everything at once.

Reading physics-driven parameters back after update (`ay2d_get_parameters`)
gives the rig's values; setting them while physics is on is overridden
on the next step. Hold the rig (`ay2d_set_physics(m, 0)`) to drive those
parameters by hand.

## 10. Performance

- Frame cost is one `set` (or `set_all`), one `update`, one `drawables`:
  three calls for any model size. Never resolve ids per frame.
- `ay2d_drawables` copies `n` small structs; the geometry behind the
  pointers is not copied.
- No allocation after load in any call; the library is safe to drive from
  a real-time thread as far as memory goes.
- Upload `uvs` and `indices` once per mesh (they never change); stream only
  `positions`. A model with 100 meshes of 50 vertices moves 40 KB a frame.
- `update` work scales with keyforms and deformers, typically well under a
  millisecond for a streaming model on one core.

## 11. Binding from other languages

The surface uses only `uint8_t`, `uint16_t`, `uint32_t`, `int32_t`,
`float`, `size_t`, `char *`, two plain structs and one opaque pointer. No
callbacks, no variadics, no bitfields, no C++.

**C# (Unity, Godot C#)**: `[DllImport("ayatsuri_capi")]` per function with
`IntPtr` for the handle and `float[]`/`uint[]` arrays (pinned by the
marshaller for the call). Read `ay2d_drawable` as a `[StructLayout(Sequential)]`
struct of the same fields; its pointers are `IntPtr` that you wrap with
`Span<float>` or copy into a `NativeArray` for the mesh.

**Python**: `ctypes.CDLL("libayatsuri_capi.so")`, set `argtypes` and
`restype` per function; `(ctypes.c_float * n)()` for arrays; drawables as a
`ctypes.Structure`. `numpy.ctypeslib.as_array(ptr, (vertex_count * 2,))`
views positions without a copy.

**Godot (GDExtension, C++)**: include the header, link the static archive,
wrap a model in a `RefCounted`; feed `positions`/`uvs`/`indices` to
`ArrayMesh` surfaces or draw with `RenderingServer` canvas items.

**Unreal**: a third-party module wrapping the static library; drawables
become a procedural mesh or a custom vertex factory streaming positions.

**Node**: `ffi-napi`/`koffi` with the same signatures; or compile the
archive into a native addon.

In every binding keep the three-call frame; the cost of a binding is per
call, not per byte.

## 12. Versioning and stability

- `AY2D_ABI_VERSION` changes only on a breaking change to an existing
  signature, struct or behaviour. Adding functions, flags or struct fields
  at the end of `ay2d_drawable` does not bump it; a host built against an
  older header keeps working against a newer library.
- The header is generated from the Rust source (`cargo xtask capi`) and a
  CI check fails when the committed copy is stale, so the header never
  describes something the library does not do.
- Status codes are only ever added.
- The library version (`ay2d_version`) follows Ayatsuri2D's release.

## 13. Building and linking

`cargo xtask capi` builds `target/capi/`:

| File | |
|---|---|
| `ayatsuri.h` | the header |
| `libayatsuri_capi.so` / `.dylib` / `ayatsuri_capi.dll` | shared, for plugin hosts |
| `libayatsuri_capi.a` / `ayatsuri_capi.lib` | static, to compile in |
| `LICENSE` | Apache-2.0 |

The library is built under the `capi-release` profile: fat LTO, symbols
stripped, one section per function. Only the `ay2d_*` entry points are
exported.

    shared   cc host.c -I. -L. -layatsuri_capi
    static   cc host.c -I. libayatsuri_capi.a -Wl,--gc-sections -lpthread -ldl -lm
             (macOS: -Wl,-dead_strip -framework CoreFoundation; MSVC: /OPT:REF)

With the archive, the host's linker drops every function the host never
reaches. `examples/host.c` is a complete host and the check CI runs.
