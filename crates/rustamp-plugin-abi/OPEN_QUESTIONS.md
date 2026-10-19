# Open questions (S09)

Ambiguities hit while implementing, plus S09 open items this crate depends on.
Nothing here was resolved from memory or outside S09.

## Spec ambiguities

1. **Output `id` type.** S09 4.1 calls it a "32-bit int" but notes a 5.x host overwrites it with a pointer. Modeled as `c_int`; a 5.x-aware host on x86 must reinterpret it.
2. **Output `write` data pointer.** S09 says "pointer to bytes"; modeled `*mut c_char`. Constness/signedness not specified.
3. **PCM / frame / row pointers** (input members 26, 28, 29, 31, 34) are "pointer to PCM/data"; modeled `*mut c_void`. Element types are not given.
4. **Input `eq_set` band pointer** ("pointer to 10 band bytes") modeled `*mut c_char`.
5. **String constness** of input `get_file_info` / `info_box` / `is_our_file` / `play` and of all `description` members is unspecified; modeled `*const c_char` (title buffer `*mut c_char`).
6. **Input `file_extensions`** is writable per 3.4 ("may be rewritten"), modeled `*mut c_char`; nullability is U-19.
7. **Input 39th member** type is "pointer to host service table"; modeled `*mut c_void`, exposed as separate `InModuleExt` since its presence depends on a version flag.
8. **DSP header** is 12 bytes in 2.x and 16 in 5.x; both are modeled (`DspHeader`, `DspHeader5x`). S09 does not say which size a host should expect for 0x20 headers.
9. **Vis waveform bytes** are stored as `u8`; signedness in 2.95 is U-3.
10. **`get_module` return** typed as pointer to the respective module struct; S09 says only "module pointer".
11. **Entry-point function types** are not defined: S09 says the arguments are none in 2.x but a main-window handle in 5.x (harmless under cdecl). Hosts should call them as `extern "C" fn() -> *mut T`.
12. **No padding** is asserted by S09 and relied upon; the layout asserts check it on x86 only.
13. **Optional extended-info exports**: only names are provided, no signatures (U-13).
14. **Vis `config` call order**: S09 5.3 item 7 is UNV; `parent_window` validity before `config` is assumed documented-but-unverified.
15. **Vis module list length**: S09 5.1 says the host asks for `get_module(0)`, `get_module(1)`, ... until the answer is null; it does not bound the list. A host needs an own bound so that a plugin which never answers null cannot hang it; the bound is host policy, not an ABI fact. The crate models the list only through the `get_module` callback, so nothing here depends on a number.
16. **Vis discovery**: U-5 (a `vis_` file-name prefix filter, or any DLL probed for the export) is still open. S09 2 records for 2.95 that *any* DLL in the Plugins folder may be probed for `winampVisGetHeader` and that no prefix filter string is present, so a host that recognizes a visualization by its export alone matches what S09 states; the `vis_` prefix stays a convention.

## S09 open items depended on

U-3 (waveform encoding), U-5 (vis discovery), U-7 (version acceptance: strict vs range), U-8 (general purpose `init` return role), U-9 (arguments to header getters), U-13 (extended-info signatures), U-16 (nullable input callbacks; all callback fields are `Option`), U-19 (null `file_extensions`).
