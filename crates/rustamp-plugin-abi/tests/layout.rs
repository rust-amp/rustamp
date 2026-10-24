//! Layout conformance of the classic plugin ABI against `specs/S09-plugin-abi.md`.
//!
//! S09 §0 fixes the layout of every classic plugin struct: a 32-bit x86
//! process, `#[repr(C)]`, members 4-byte aligned, no padding. The member
//! tables of S09 §3.1 (`In_Module`), §4.1 (`Out_Module`), §5.1/§5.2
//! (visualization header and module), §6.1/§6.2 (DSP header and module) and
//! §7.1 (general purpose plugin) give, for every member, its offset in that
//! layout and its neutral type.
//!
//! This file transcribes those tables **once, as data**, and checks them
//! against the types of `rustamp-plugin-abi`:
//!
//! * **On every target**: the members must have the widths and the order the
//!   spec's tables give, and the type must have the size and the alignment a C
//!   compiler gives a struct with those members. A member that moves, grows,
//!   shrinks or disappears breaks this.
//! * **On 32-bit x86** (the target the ABI is defined for): the offsets must be
//!   the spec's offsets byte for byte, the size must be the spec's size, the
//!   alignment must be 4, and the spec's offsets must leave no padding.
//!
//! Three independent mechanisms are used, so that no single mistake can hide:
//!
//! 1. `offset_of!` - what the compiler says about the type.
//! 2. Pointer arithmetic on a live instance - what the memory image really
//!    holds, checked against (1).
//! 3. The crate's own source - the order in which the members are *declared*,
//!    checked on its own, so that a member that swapped places is reported as
//!    a swap and not only as a member at the wrong offset.
//!
//! The spec's member names are neutral descriptive names that are not part of
//! the ABI (S09 §0); they are recorded here next to the field names so that
//! the table stays a faithful copy of the spec.

use std::ffi::c_void;
use std::mem::{align_of, size_of};

use rustamp_plugin_abi::dsp::{DspHeader, DspHeader5x, DspModule};
use rustamp_plugin_abi::general::GeneralPlugin;
use rustamp_plugin_abi::input::{InModule, InModuleExt};
use rustamp_plugin_abi::output::OutModule;
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

/// The width class of one member, as S09's "Neutral type" column states it.
#[derive(Clone, Copy, Debug)]
enum Width {
    /// "signed 32-bit int", "int", "int (0/1)", "int (bit flags)" and
    /// "32-bit int": four bytes on every target.
    Int32,
    /// "pointer to ANSI string", "window handle", "module handle",
    /// "function (...)" and "pointer": a pointer, four bytes wide in the
    /// 32-bit x86 layout S09 defines.
    Pointer,
    /// A byte array, e.g. "2 x 576 unsigned bytes": `n` bytes, aligned to one.
    Bytes(usize),
    /// A struct embedded by value (the 38 common members of the input
    /// module): its own size and alignment.
    Embed(&'static Layout),
}

impl Width {
    /// The size and the alignment such a member has on the target the test
    /// runs on.
    fn size_align(self) -> (usize, usize) {
        match self {
            Width::Int32 => (4, 4),
            Width::Pointer => (size_of::<*const c_void>(), align_of::<*const c_void>()),
            Width::Bytes(len) => (len, 1),
            Width::Embed(inner) => {
                let (_, size, align) = c_layout(inner.members);
                (size, align)
            }
        }
    }

    /// The size such a member has in the 32-bit x86 layout S09 defines.
    fn x86_size(self) -> usize {
        match self {
            Width::Int32 => 4,
            Width::Pointer => 4,
            Width::Bytes(len) => len,
            Width::Embed(inner) => inner.x86_size,
        }
    }

    /// The alignment such a member has in the 32-bit x86 layout S09 defines.
    fn x86_align(self) -> usize {
        match self {
            Width::Int32 => 4,
            Width::Pointer => 4,
            Width::Bytes(_) => 1,
            Width::Embed(inner) => inner.x86_align,
        }
    }
}

/// One member of a spec layout table.
#[derive(Clone, Copy, Debug)]
struct Member {
    /// The field name in `rustamp-plugin-abi`.
    name: &'static str,
    /// The neutral name S09's table gives for this member.
    spec_name: &'static str,
    /// The width class S09's "Neutral type" column gives.
    width: Width,
    /// The offset in the 32-bit x86 layout, from S09's "Offset" column.
    x86_offset: usize,
}

/// One struct of the layout table, with what the type really is.
#[derive(Debug)]
struct Layout {
    /// The Rust type, for the failure messages.
    type_name: &'static str,
    /// The file of this crate that declares the type.
    source_file: &'static str,
    /// The S09 section the table comes from.
    spec: &'static str,
    /// The size the type has on the target the test runs on.
    size: usize,
    /// The alignment the type has on the target the test runs on.
    align: usize,
    /// The size in the 32-bit x86 layout, from S09.
    x86_size: usize,
    /// The alignment in the 32-bit x86 layout: 4 for every struct S09
    /// describes (S09 §0: "Members are 4-byte aligned").
    x86_align: usize,
    /// The members, in the order of S09's table.
    members: &'static [Member],
    /// The offsets `offset_of!` reports for the type.
    offsets: &'static [usize],
}

/// Declares one struct of the layout table and the test that checks it.
macro_rules! abi_layout {
    (
        name = $name:ident,
        test = $test:ident,
        image = $image:ident,
        ty = $ty:ty,
        source = $source:literal,
        spec = $spec:literal,
        size = $size:expr,
        align = $align:expr,
        members = {
            $( $field:ident => ($spec_name:literal, $width:expr, $x86_offset:expr) ),* $(,)?
        }
    ) => {
        /// The layout table of this struct, straight from the spec.
        static $name: Layout = Layout {
            type_name: stringify!($ty),
            source_file: $source,
            spec: $spec,
            size: core::mem::size_of::<$ty>(),
            align: core::mem::align_of::<$ty>(),
            x86_size: $size,
            x86_align: $align,
            members: &[
                $(Member {
                    name: stringify!($field),
                    spec_name: $spec_name,
                    width: $width,
                    x86_offset: $x86_offset,
                },)*
            ],
            offsets: &[$(core::mem::offset_of!($ty, $field)),*],
        };

        /// The offsets the memory image of the type really has, computed with
        /// pointer arithmetic: an independent check of what `offset_of!`
        /// reports.
        fn $image(module: *const $ty) -> Vec<usize> {
            let base = module as usize;
            vec![$({
                // SAFETY: the caller passes the address of a live instance of
                // this type; only the address of the member is taken, no value
                // is read and nothing is written.
                let member = unsafe { core::ptr::addr_of!((*module).$field) };
                member as usize - base
            }),*]
        }

        /// The layout of this struct matches the spec.
        #[test]
        fn $test() {
            let module = core::mem::MaybeUninit::<$ty>::uninit();
            check_layout(&$name, &$image(module.as_ptr()));
        }
    };
}

abi_layout! {
    name = IN_MODULE,
    test = in_module_layout,
    image = in_module_image_offsets,
    ty = InModule,
    source = "src/input.rs",
    spec = "S09 §3.1",
    size = 0x98,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        main_window => ("main_window", Width::Pointer, 0x08),
        dll_instance => ("dll_instance", Width::Pointer, 0x0C),
        file_extensions => ("file_extensions", Width::Pointer, 0x10),
        seekable => ("seekable", Width::Int32, 0x14),
        flags => ("flags", Width::Int32, 0x18),
        config => ("config", Width::Pointer, 0x1C),
        about => ("about", Width::Pointer, 0x20),
        init => ("init", Width::Pointer, 0x24),
        quit => ("quit", Width::Pointer, 0x28),
        get_file_info => ("get_file_info", Width::Pointer, 0x2C),
        info_box => ("info_box", Width::Pointer, 0x30),
        is_our_file => ("is_our_file", Width::Pointer, 0x34),
        play => ("play", Width::Pointer, 0x38),
        pause => ("pause", Width::Pointer, 0x3C),
        unpause => ("unpause", Width::Pointer, 0x40),
        is_paused => ("is_paused", Width::Pointer, 0x44),
        stop => ("stop", Width::Pointer, 0x48),
        get_length => ("get_length", Width::Pointer, 0x4C),
        get_position => ("get_position", Width::Pointer, 0x50),
        seek => ("seek", Width::Pointer, 0x54),
        set_volume => ("set_volume", Width::Pointer, 0x58),
        set_pan => ("set_pan", Width::Pointer, 0x5C),
        analyzer_init => ("analyzer_init", Width::Pointer, 0x60),
        analyzer_deinit => ("analyzer_deinit", Width::Pointer, 0x64),
        analyzer_feed_pcm => ("analyzer_feed_pcm", Width::Pointer, 0x68),
        analyzer_get_mode => ("analyzer_get_mode", Width::Pointer, 0x6C),
        analyzer_feed_rows => ("analyzer_feed_rows", Width::Pointer, 0x70),
        vis_feed_pcm => ("vis_feed_pcm", Width::Pointer, 0x74),
        vis_get_mode => ("vis_get_mode", Width::Pointer, 0x78),
        vis_feed_frame => ("vis_feed_frame", Width::Pointer, 0x7C),
        vis_set_format => ("vis_set_format", Width::Pointer, 0x80),
        dsp_is_active => ("dsp_is_active", Width::Pointer, 0x84),
        dsp_process => ("dsp_process", Width::Pointer, 0x88),
        eq_set => ("eq_set", Width::Pointer, 0x8C),
        set_info => ("set_info", Width::Pointer, 0x90),
        out_module => ("out_module", Width::Pointer, 0x94),
    }
}

abi_layout! {
    name = IN_MODULE_EXT,
    test = in_module_ext_layout,
    image = in_module_ext_image_offsets,
    ty = InModuleExt,
    source = "src/input.rs",
    spec = "S09 §3.1 (5.x-only 39th member at 0x98; the size follows from it)",
    size = 0x9C,
    align = 4,
    members = {
        base => ("the 38 common members", Width::Embed(&IN_MODULE), 0x00),
        host_services => ("host service table", Width::Pointer, 0x98),
    }
}

abi_layout! {
    name = OUT_MODULE,
    test = out_module_layout,
    image = out_module_image_offsets,
    ty = OutModule,
    source = "src/output.rs",
    spec = "S09 §4.1",
    size = 0x50,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        id => ("id", Width::Int32, 0x08),
        main_window => ("main_window", Width::Pointer, 0x0C),
        dll_instance => ("dll_instance", Width::Pointer, 0x10),
        config => ("config", Width::Pointer, 0x14),
        about => ("about", Width::Pointer, 0x18),
        init => ("init", Width::Pointer, 0x1C),
        quit => ("quit", Width::Pointer, 0x20),
        open => ("open", Width::Pointer, 0x24),
        close => ("close", Width::Pointer, 0x28),
        write => ("write", Width::Pointer, 0x2C),
        can_write => ("can_write", Width::Pointer, 0x30),
        is_playing => ("is_playing", Width::Pointer, 0x34),
        pause => ("pause", Width::Pointer, 0x38),
        set_volume => ("set_volume", Width::Pointer, 0x3C),
        set_pan => ("set_pan", Width::Pointer, 0x40),
        flush => ("flush", Width::Pointer, 0x44),
        get_played_ms => ("get_played_ms", Width::Pointer, 0x48),
        get_written_ms => ("get_written_ms", Width::Pointer, 0x4C),
    }
}

abi_layout! {
    name = VIS_HEADER,
    test = vis_header_layout,
    image = vis_header_image_offsets,
    ty = VisHeader,
    source = "src/vis.rs",
    spec = "S09 §5.1",
    size = 12,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        get_module => ("get_module", Width::Pointer, 0x08),
    }
}

abi_layout! {
    name = VIS_MODULE,
    test = vis_module_layout,
    image = vis_module_image_offsets,
    ty = VisModule,
    source = "src/vis.rs",
    spec = "S09 §5.2",
    size = 0x938,
    align = 4,
    members = {
        description => ("description", Width::Pointer, 0x000),
        parent_window => ("parent_window", Width::Pointer, 0x004),
        dll_instance => ("dll_instance", Width::Pointer, 0x008),
        sample_rate => ("sample_rate", Width::Int32, 0x00C),
        channels => ("channels", Width::Int32, 0x010),
        latency_ms => ("latency_ms", Width::Int32, 0x014),
        delay_ms => ("delay_ms", Width::Int32, 0x018),
        spectrum_rows => ("spectrum_rows", Width::Int32, 0x01C),
        waveform_rows => ("waveform_rows", Width::Int32, 0x020),
        spectrum_data => ("spectrum_data", Width::Bytes(1152), 0x024),
        waveform_data => ("waveform_data", Width::Bytes(1152), 0x4A4),
        config => ("config", Width::Pointer, 0x924),
        init => ("init", Width::Pointer, 0x928),
        render => ("render", Width::Pointer, 0x92C),
        quit => ("quit", Width::Pointer, 0x930),
        user_data => ("user_data", Width::Pointer, 0x934),
    }
}

abi_layout! {
    name = DSP_HEADER,
    test = dsp_header_layout,
    image = dsp_header_image_offsets,
    ty = DspHeader,
    source = "src/dsp.rs",
    spec = "S09 §6.1 (2.x header)",
    size = 12,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        get_module => ("get_module", Width::Pointer, 0x08),
    }
}

abi_layout! {
    name = DSP_HEADER_5X,
    test = dsp_header_5x_layout,
    image = dsp_header_5x_image_offsets,
    ty = DspHeader5x,
    source = "src/dsp.rs",
    spec = "S09 §6.1 (5.x header)",
    size = 16,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        get_module => ("get_module", Width::Pointer, 0x08),
        key_check => ("key_check", Width::Pointer, 0x0C),
    }
}

abi_layout! {
    name = DSP_MODULE,
    test = dsp_module_layout,
    image = dsp_module_image_offsets,
    ty = DspModule,
    source = "src/dsp.rs",
    spec = "S09 §6.2",
    size = 32,
    align = 4,
    members = {
        description => ("description", Width::Pointer, 0x00),
        parent_window => ("parent_window", Width::Pointer, 0x04),
        dll_instance => ("dll_instance", Width::Pointer, 0x08),
        config => ("config", Width::Pointer, 0x0C),
        init => ("init", Width::Pointer, 0x10),
        process => ("process", Width::Pointer, 0x14),
        quit => ("quit", Width::Pointer, 0x18),
        user_data => ("user_data", Width::Pointer, 0x1C),
    }
}

abi_layout! {
    name = GENERAL_PLUGIN,
    test = general_plugin_layout,
    image = general_plugin_image_offsets,
    ty = GeneralPlugin,
    source = "src/general.rs",
    spec = "S09 §7.1",
    size = 28,
    align = 4,
    members = {
        version => ("version", Width::Int32, 0x00),
        description => ("description", Width::Pointer, 0x04),
        init => ("init", Width::Pointer, 0x08),
        config => ("config", Width::Pointer, 0x0C),
        quit => ("quit", Width::Pointer, 0x10),
        parent_window => ("parent_window", Width::Pointer, 0x14),
        dll_instance => ("dll_instance", Width::Pointer, 0x18),
    }
}

/// Every struct of the table.
const ALL: &[&Layout] = &[
    &IN_MODULE,
    &IN_MODULE_EXT,
    &OUT_MODULE,
    &VIS_HEADER,
    &VIS_MODULE,
    &DSP_HEADER,
    &DSP_HEADER_5X,
    &DSP_MODULE,
    &GENERAL_PLUGIN,
];

/// Every source file of this crate, so that the set of declared structs can
/// be compared with the table.
const SOURCE_FILES: &[&str] = &[
    "src/common.rs",
    "src/dsp.rs",
    "src/general.rs",
    "src/input.rs",
    "src/lib.rs",
    "src/output.rs",
    "src/vis.rs",
];

/// The offsets, the size and the alignment a C compiler gives a struct with
/// these members, on the target the test runs on (S09 §0: `#[repr(C)]`, every
/// member at its own alignment, the struct aligned to its widest member).
fn c_layout(members: &[Member]) -> (Vec<usize>, usize, usize) {
    let mut offset: usize = 0;
    let mut align: usize = 1;
    let mut offsets = Vec::with_capacity(members.len());
    for member in members {
        let (size, member_align) = member.width.size_align();
        offset = offset.next_multiple_of(member_align);
        offsets.push(offset);
        offset += size;
        align = align.max(member_align);
    }
    (offsets, offset.next_multiple_of(align), align)
}

/// Checks one struct of the table against the spec.
fn check_layout(layout: &Layout, image_offsets: &[usize]) {
    let (expected_offsets, expected_size, expected_align) = c_layout(layout.members);

    assert_eq!(
        layout.offsets.len(),
        layout.members.len(),
        "{}: the table and the type disagree about the member count",
        layout.type_name
    );
    assert_eq!(
        image_offsets.len(),
        layout.members.len(),
        "{}: the memory image and the table disagree about the member count",
        layout.type_name
    );

    for (index, member) in layout.members.iter().enumerate() {
        assert_eq!(
            layout.offsets[index],
            expected_offsets[index],
            "{}::{} ({} \"{}\") must sit at byte offset {} on this target, but sits at {}",
            layout.type_name,
            member.name,
            layout.spec,
            member.spec_name,
            expected_offsets[index],
            layout.offsets[index]
        );
        assert_eq!(
            image_offsets[index],
            layout.offsets[index],
            "{}::{} ({} \"{}\") is at byte offset {} in the memory image, but the compiler says {}",
            layout.type_name,
            member.name,
            layout.spec,
            member.spec_name,
            image_offsets[index],
            layout.offsets[index]
        );
    }

    assert_eq!(
        layout.size, expected_size,
        "{} must be {} bytes on this target",
        layout.type_name, expected_size
    );
    assert_eq!(
        layout.align, expected_align,
        "{} must be aligned to {} bytes on this target",
        layout.type_name, expected_align
    );

    // The spec's own table must leave no padding (S09 §0: "there is no padding
    // anywhere in the structures below"). Every member starts where the
    // previous one ended.
    let mut offset = 0;
    for member in layout.members {
        assert_eq!(
            member.x86_offset, offset,
            "{}::{} ({} \"{}\") must sit at 0x{:02X}, but the spec's table leaves a gap",
            layout.type_name, member.name, layout.spec, member.spec_name, offset
        );
        offset += member.width.x86_size();
    }
    assert_eq!(
        offset, layout.x86_size,
        "{} must be {} bytes in the 32-bit x86 layout",
        layout.type_name, layout.x86_size
    );

    // The alignment of the struct is the alignment of its widest member
    // (S09 §0: "Members are 4-byte aligned").
    let mut x86_align: usize = 1;
    for member in layout.members {
        x86_align = x86_align.max(member.width.x86_align());
    }
    assert_eq!(
        x86_align, layout.x86_align,
        "{} must be aligned to {} bytes in the 32-bit x86 layout",
        layout.type_name, layout.x86_align
    );

    // On the 32-bit x86 target the ABI is defined for, the offsets must be
    // the spec's offsets byte for byte.
    #[cfg(target_arch = "x86")]
    {
        assert_eq!(
            layout.size, layout.x86_size,
            "{} must be {} bytes on 32-bit x86",
            layout.type_name, layout.x86_size
        );
        assert_eq!(
            layout.align, layout.x86_align,
            "{} must be aligned to {} bytes on 32-bit x86",
            layout.type_name, layout.x86_align
        );
        for (index, member) in layout.members.iter().enumerate() {
            assert_eq!(
                layout.offsets[index],
                member.x86_offset,
                "{}::{} ({} \"{}\") must sit at 0x{:02X} on 32-bit x86, but sits at 0x{:02X}",
                layout.type_name,
                member.name,
                layout.spec,
                member.spec_name,
                member.x86_offset,
                layout.offsets[index]
            );
        }
    }
}

/// The source of this crate, per file, so that the declaration order can be
/// checked without a file system at run time.
fn source_for(file: &str) -> &'static str {
    match file {
        "src/common.rs" => include_str!("../src/common.rs"),
        "src/dsp.rs" => include_str!("../src/dsp.rs"),
        "src/general.rs" => include_str!("../src/general.rs"),
        "src/input.rs" => include_str!("../src/input.rs"),
        "src/lib.rs" => include_str!("../src/lib.rs"),
        "src/output.rs" => include_str!("../src/output.rs"),
        "src/vis.rs" => include_str!("../src/vis.rs"),
        other => panic!("no source of this crate is registered for {other}"),
    }
}

/// The names of the structs a source declares.
fn declared_structs(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.trim().strip_prefix("pub struct ") else {
            continue;
        };
        let name = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('{')
            .trim();
        if !name.is_empty() {
            names.push(name.to_string());
        }
    }
    names
}

/// The field names of `struct_name` in declaration order, read from the
/// crate's own source.
fn declared_fields(source: &str, struct_name: &str) -> Vec<String> {
    let header = format!("pub struct {struct_name} {{");
    let start = source
        .lines()
        .position(|line| line.trim_end() == header)
        .unwrap_or_else(|| panic!("{struct_name} is not declared in this source"));
    let mut fields = Vec::new();
    for line in source.lines().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed == "}" {
            break;
        }
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        let Some((name, _)) = rest.split_once(':') else {
            continue;
        };
        fields.push(name.trim().to_string());
    }
    fields
}

/// The members of every struct are declared in the order of the spec's
/// tables. The offsets are checked member by member already; this check names
/// the swap directly, so that a member that changed places is not reported
/// only as "this member sits at the wrong offset".
#[test]
fn the_members_are_declared_in_the_spec_order() {
    for layout in ALL {
        let declared = declared_fields(source_for(layout.source_file), layout.type_name);
        let expected: Vec<&str> = layout.members.iter().map(|member| member.name).collect();
        assert_eq!(
            declared, expected,
            "{} ({}) must declare its members in the order of the spec's table",
            layout.type_name, layout.spec
        );
    }
}

/// Every struct this crate declares is in the table, and the table names no
/// struct the crate does not declare. A struct added to the crate without a
/// table entry fails here.
#[test]
fn the_table_covers_every_struct_of_the_crate() {
    let mut declared: Vec<String> = Vec::new();
    for file in SOURCE_FILES {
        declared.extend(declared_structs(source_for(file)));
    }
    declared.sort();
    let mut covered: Vec<&str> = ALL.iter().map(|layout| layout.type_name).collect();
    covered.sort();
    assert_eq!(
        declared, covered,
        "the layout table must cover exactly the structs of the crate"
    );
}
