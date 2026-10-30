/*
 * bindgen input.
 *
 * Only `libraw_types.h` is included, and deliberately so. `libraw.h` is a C++
 * header — it declares `class LibRaw` alongside the C API — and pointing
 * bindgen at it drags the whole C++ surface in. The struct layout is the one
 * thing we cannot safely hand-write, so bindgen generates that; the C function
 * signatures are stable and short enough to declare by hand in Rust.
 */
#include <libraw/libraw_types.h>
