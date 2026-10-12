/*
 * host.c: a C program that drives the whole ABI the way an engine would,
 * and the end-to-end check `cargo xtask capi` runs against the built
 * library. Usage: host model.aom
 *
 *   cc host.c -I../include -L../../../target/capi-release \
 *      -layatsuri_capi -lpthread -ldl -lm -o host
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ayatsuri.h"

static int fail(const char *what) {
    fprintf(stderr, "host: %s: %s\n", what, ay2d_last_error());
    return 1;
}

static unsigned char *slurp(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc(n > 0 ? (size_t)n : 1);
    if (buf && fread(buf, 1, (size_t)n, f) != (size_t)n) { free(buf); buf = NULL; }
    fclose(f);
    *len = (size_t)n;
    return buf;
}

int main(int argc, char **argv) {
    if (ay2d_abi_version() != AY2D_ABI_VERSION) return fail("abi version");
    printf("ayatsuri %s, abi %u\n", ay2d_version(), ay2d_abi_version());

    /* garbage is refused, with a reason, and nothing crashes */
    if (ay2d_load((const uint8_t *)"nope", 4) != NULL) return fail("garbage loaded");
    if (strlen(ay2d_last_error()) == 0) return fail("no reason given");
    ay2d_free(NULL);
    if (argc < 2) { printf("error path ok; pass a model.aom to drive one\n"); return 0; }

    size_t len = 0;
    unsigned char *bytes = slurp(argv[1], &len);
    if (!bytes) { perror(argv[1]); return 1; }
    ay2d_model *m = ay2d_load(bytes, len);
    free(bytes); /* the model keeps nothing of the host's buffer */
    if (!m) return fail("load");

    uint32_t np = ay2d_parameter_count(m), nparts = ay2d_part_count(m);
    uint32_t nmesh = ay2d_mesh_count(m), ntex = ay2d_texture_count(m);
    ay2d_canvas_info canvas;
    if (ay2d_canvas(m, &canvas) != AY2D_STATUS_OK) return fail("canvas");
    printf("%u parameters, %u parts, %u meshes, %u textures, canvas %.0fx%.0f\n",
           np, nparts, nmesh, ntex, canvas.width, canvas.height);
    for (uint32_t i = 0; i < np; i++) printf("  param %u %s\n", i, ay2d_parameter_id(m, i));

    /* ranges, then the whole pose as one array */
    float *lo = calloc(np, sizeof(float)), *hi = calloc(np, sizeof(float));
    float *def = calloc(np, sizeof(float)), *pose = calloc(np, sizeof(float));
    if (ay2d_parameter_ranges(m, lo, hi, def, np) != AY2D_STATUS_OK) return fail("ranges");
    if (ay2d_get_parameters(m, pose, np) != AY2D_STATUS_OK) return fail("get");
    for (uint32_t i = 0; i < np; i++)
        if (pose[i] != def[i]) return fail("a fresh model is not at its defaults");

    int32_t a = ay2d_parameter_index(m, "ParamA");
    if (a < 0) return fail("ParamA missing");
    uint32_t idx[1] = { (uint32_t)a };
    float val[1] = { hi[a] };
    if (ay2d_set_parameters(m, idx, val, 1) != AY2D_STATUS_OK) return fail("set");
    if (ay2d_update(m, 1.0f / 60.0f) != AY2D_STATUS_OK) return fail("update");

    /* drawables: ask for the count, then the views */
    uint32_t n = ay2d_drawables(m, NULL, 0);
    ay2d_drawable *d = calloc(n ? n : 1, sizeof(ay2d_drawable));
    if (ay2d_drawables(m, d, n) != n) return fail("drawables");
    for (uint32_t i = 0; i < n; i++)
        printf("  draw mesh %u (%s): %u verts, %u indices, texture %u, opacity %.2f, first xy %.3f %.3f\n",
               d[i].mesh, ay2d_mesh_id(m, d[i].mesh), d[i].vertex_count, d[i].index_count,
               d[i].texture, d[i].opacity, d[i].positions[0], d[i].positions[1]);
    if (n == 0) return fail("nothing to draw");

    /* geometry for a GPU host, a texture page, physics switches */
    ay2d_drawable g;
    if (ay2d_mesh_geometry(m, 0, &g) != AY2D_STATUS_OK) return fail("geometry");
    const uint8_t *png; size_t png_len;
    if (ntex && ay2d_texture(m, 0, &png, &png_len) != AY2D_STATUS_OK) return fail("texture");
    printf("  texture 0: %zu bytes\n", ntex ? png_len : 0);
    if (ay2d_has_physics(m)) {
        ay2d_set_wind(m, 0.2f, 0.0f);
        for (int f = 0; f < 60; f++) {
            val[0] = (f / 10) % 2 ? lo[a] : hi[a];
            ay2d_set_parameters(m, idx, val, 1);
            if (ay2d_update(m, 1.0f / 60.0f) != AY2D_STATUS_OK) return fail("physics update");
        }
        ay2d_reset_physics(m);
        ay2d_set_physics(m, 0);
    }
    /* out of range is a status, never a crash */
    if (ay2d_mesh_geometry(m, nmesh + 5, &g) != AY2D_STATUS_RANGE) return fail("range check");
    if (ay2d_set_all_parameters(m, pose, 0) != AY2D_STATUS_TOO_SMALL && np > 0) return fail("size check");

    free(lo); free(hi); free(def); free(pose); free(d);
    ay2d_free(m);
    printf("ok\n");
    return 0;
}
