use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

use heck::{ToSnakeCase, ToUpperCamelCase};

use codegen_shared::naming::{
    c_add_listener_symbol, c_constructor_symbol, c_free_symbol, c_list_field,
    c_list_release_symbol, c_list_type_name, c_method_symbol, c_native_object_symbol,
    c_remove_listener_symbol, c_type_name, constructor_suffix, is_binding_accessor,
    listed_classes, struct_has_owned_fields, TypeOrigins, STRING_FREE_FN, STRING_LIST_FREE_FN,
    STRING_MAP_FREE_FN,
};
use codegen_shared::ir::{
    Api, Class, Constructor, EventGroup, Header, Method, Param, Struct, TypeRef,
};
use codegen_shared::GeneratedFile;

/// Per-file collection of DllImport declarations and callback delegate types,
/// emitted together at the bottom of the file.
#[derive(Default)]
struct FileCtx {
    externs: BTreeSet<String>,
    delegates: BTreeMap<String, String>,
}

pub fn generate(
    api: &Api,
    header: &Header,
    _origins: &TypeOrigins,
    csharp_out: &Path,
    prefix: &str,
    subdir: Option<&Path>,
) -> GeneratedFile {
    let file_name = format!("{}.cs", header.stem.to_upper_camel_case());
    let out_path = match subdir {
        Some(dir) => {
            let mut path = csharp_out.to_path_buf();
            for part in dir.components() {
                path.push(part.as_os_str().to_string_lossy().to_upper_camel_case());
            }
            path.join(&file_name)
        }
        None => csharp_out.join(&file_name),
    };
    GeneratedFile {
        path: out_path,
        contents: generate_csharp(api, header, prefix),
    }
}

/// Shared runtime pieces every generated file leans on: the library name, the
/// delegate keeper, and the string list/map interop helpers.
pub fn generate_support(csharp_out: &Path) -> GeneratedFile {
    let mut out = String::new();
    banner(&mut out);
    writeln!(
        out,
        r#"internal static class Libraries
{{
    internal const string NativeApi = "nativeapi";
}}

/// <summary>
/// Keeps callback delegates alive for the lifetime of the process: the C ABI
/// stores the function pointer but offers no hook to release it.
/// </summary>
internal static class CallbackKeeper
{{
    private static readonly List<Delegate> Retained = new();

    internal static T Retain<T>(T callback) where T : Delegate
    {{
        lock (Retained)
        {{
            Retained.Add(callback);
        }}
        return callback;
    }}
}}

[StructLayout(LayoutKind.Sequential)]
internal struct native_string_list_t
{{
    internal IntPtr items;
    internal CLong count;
}}

[StructLayout(LayoutKind.Sequential)]
internal struct native_string_map_t
{{
    internal IntPtr keys;
    internal IntPtr values;
    internal CLong count;
}}

internal static partial class Interop
{{
    [DllImport(Libraries.NativeApi, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void {STRING_FREE_FN}(IntPtr str);

    [DllImport(Libraries.NativeApi, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void {STRING_LIST_FREE_FN}(ref native_string_list_t list);

    [DllImport(Libraries.NativeApi, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void {STRING_MAP_FREE_FN}(ref native_string_map_t map);

    /// <summary>Reads an owned C string and frees it.</summary>
    internal static string? ConsumeString(IntPtr value)
    {{
        if (value == IntPtr.Zero)
        {{
            return null;
        }}
        try
        {{
            return Marshal.PtrToStringUTF8(value);
        }}
        finally
        {{
            {STRING_FREE_FN}(value);
        }}
    }}

    /// <summary>Reads an owned C string list and frees it.</summary>
    internal static string[] ConsumeStringList(ref native_string_list_t list)
    {{
        var count = list.items == IntPtr.Zero ? 0 : checked((int)list.count.Value);
        var items = new string[count];
        for (var i = 0; i < count; i++)
        {{
            var ptr = Marshal.ReadIntPtr(list.items, i * IntPtr.Size);
            items[i] = ptr == IntPtr.Zero ? string.Empty : Marshal.PtrToStringUTF8(ptr) ?? string.Empty;
        }}
        {STRING_LIST_FREE_FN}(ref list);
        return items;
    }}

    /// <summary>Reads an owned C string map and frees it.</summary>
    internal static Dictionary<string, string> ConsumeStringMap(ref native_string_map_t map)
    {{
        var count = map.keys == IntPtr.Zero || map.values == IntPtr.Zero
            ? 0
            : checked((int)map.count.Value);
        var entries = new Dictionary<string, string>(count);
        for (var i = 0; i < count; i++)
        {{
            var keyPtr = Marshal.ReadIntPtr(map.keys, i * IntPtr.Size);
            if (keyPtr == IntPtr.Zero)
            {{
                continue;
            }}
            var valuePtr = Marshal.ReadIntPtr(map.values, i * IntPtr.Size);
            var value = valuePtr == IntPtr.Zero ? string.Empty : Marshal.PtrToStringUTF8(valuePtr) ?? string.Empty;
            entries[Marshal.PtrToStringUTF8(keyPtr) ?? string.Empty] = value;
        }}
        {STRING_MAP_FREE_FN}(ref map);
        return entries;
    }}

    /// <summary>Copies strings into unmanaged UTF-8 buffers.</summary>
    internal static IntPtr[] AllocUtf8Array(IReadOnlyList<string> values)
    {{
        var ptrs = new IntPtr[values.Count];
        for (var i = 0; i < values.Count; i++)
        {{
            ptrs[i] = Marshal.StringToCoTaskMemUTF8(values[i]);
        }}
        return ptrs;
    }}

    /// <summary>Copies a pointer array into one unmanaged block.</summary>
    internal static IntPtr AllocPointerArray(IntPtr[] values)
    {{
        var block = Marshal.AllocHGlobal(IntPtr.Size * Math.Max(values.Length, 1));
        Marshal.Copy(values, 0, block, values.Length);
        return block;
    }}

    internal static void FreeUtf8Array(IntPtr[] values)
    {{
        foreach (var ptr in values)
        {{
            if (ptr != IntPtr.Zero)
            {{
                Marshal.FreeCoTaskMem(ptr);
            }}
        }}
    }}
}}"#
    )
    .unwrap();

    GeneratedFile {
        path: csharp_out.join("Support.cs"),
        contents: out,
    }
}

fn banner(out: &mut String) {
    writeln!(out, "// AUTO-GENERATED. DO NOT EDIT.").unwrap();
    writeln!(
        out,
        "// Any manual changes WILL BE LOST when this file is regenerated."
    )
    .unwrap();
    writeln!(out, "#nullable enable").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "using System;").unwrap();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out, "using System.Runtime.InteropServices;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "namespace NativeAPI;").unwrap();
    writeln!(out).unwrap();
}

fn generate_csharp(api: &Api, header: &Header, prefix: &str) -> String {
    let mut out = String::new();
    let mut ctx = FileCtx::default();
    banner(&mut out);

    for item in &header.enums {
        generate_enum(&mut out, item);
    }

    for item in &header.structs {
        generate_struct(&mut out, &mut ctx, item, prefix);
    }

    for group in &header.events {
        generate_event(&mut out, &mut ctx, group, prefix);
    }

    let listed = listed_classes(api);
    for class in &header.classes {
        if listed.contains(&class.name) {
            generate_list_struct(&mut out, &mut ctx, &class.name, prefix);
        }
        if class.is_instance() {
            generate_handle_class(&mut out, &mut ctx, api, header, class, prefix);
        } else {
            generate_singleton_class(&mut out, &mut ctx, api, header, class, prefix);
        }
    }

    for signature in ctx.delegates.values() {
        writeln!(out, "{signature}").unwrap();
        writeln!(out).unwrap();
    }

    if !ctx.externs.is_empty() {
        writeln!(out, "internal static partial class Interop").unwrap();
        writeln!(out, "{{").unwrap();
        let mut first = true;
        for decl in &ctx.externs {
            if !first {
                writeln!(out).unwrap();
            }
            first = false;
            writeln!(
                out,
                "    [DllImport(Libraries.NativeApi, CallingConvention = CallingConvention.Cdecl)]"
            )
            .unwrap();
            writeln!(out, "    {decl}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ---------------------------------------------------------------------------
// Enums, structs, events
// ---------------------------------------------------------------------------

fn generate_enum(out: &mut String, item: &codegen_shared::ir::Enum) {
    writeln!(out, "public enum {}", item.name).unwrap();
    writeln!(out, "{{").unwrap();
    for variant in &item.variants {
        writeln!(out, "    {} = {},", enum_member(&variant.name), variant.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_struct(out: &mut String, ctx: &mut FileCtx, item: &Struct, prefix: &str) {
    let c_ty = c_type_name(prefix, &item.name);

    // Raw C mirror.
    writeln!(out, "[StructLayout(LayoutKind.Sequential)]").unwrap();
    writeln!(out, "internal struct {c_ty}").unwrap();
    writeln!(out, "{{").unwrap();
    for field in &item.fields {
        let raw_name = field.name.to_snake_case();
        match field.ty.unwrap_optional() {
            TypeRef::Callback { params } => {
                let delegate = struct_callback_delegate(ctx, item, field, params, prefix);
                // Marshalled as a bare function pointer plus its context slot.
                let _ = delegate;
                writeln!(out, "    internal IntPtr {raw_name};").unwrap();
                writeln!(
                    out,
                    "    internal IntPtr {};",
                    codegen_shared::naming::c_user_data_param(&field.name)
                )
                .unwrap();
            }
            other => {
                writeln!(out, "    internal {} {raw_name};", cs_raw_field_type(other, prefix))
                    .unwrap();
            }
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Public form.
    writeln!(out, "public struct {}", item.name).unwrap();
    writeln!(out, "{{").unwrap();
    for field in &item.fields {
        writeln!(
            out,
            "    public {} {};",
            cs_struct_field_type(&field.ty),
            pascal(&field.name)
        )
        .unwrap();
    }
    writeln!(out).unwrap();
    write!(out, "    public {}(", item.name).unwrap();
    for (index, field) in item.fields.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").unwrap();
        }
        write!(
            out,
            "{} {}",
            cs_struct_field_type(&field.ty),
            cs_ident(&field.name)
        )
        .unwrap();
    }
    writeln!(out, ")").unwrap();
    writeln!(out, "    {{").unwrap();
    for field in &item.fields {
        writeln!(out, "        {} = {};", pascal(&field.name), cs_ident(&field.name)).unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    // FromRaw: the borrowed view of a C value.
    writeln!(out, "    internal static {} FromRaw(in {c_ty} raw)", item.name).unwrap();
    writeln!(out, "    {{").unwrap();
    write!(out, "        return new {}(", item.name).unwrap();
    for (index, field) in item.fields.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").unwrap();
        }
        let raw = format!("raw.{}", field.name.to_snake_case());
        let value = match field.ty.unwrap_optional() {
            TypeRef::Bool => format!("{raw} != 0"),
            TypeRef::String | TypeRef::CString => format!("Marshal.PtrToStringUTF8({raw})"),
            TypeRef::Enum { .. } => raw,
            TypeRef::Struct { name, .. } => format!("{name}.FromRaw(in {raw})"),
            TypeRef::Callback { .. } => "null".to_string(),
            TypeRef::Int { name } if int_needs_conv(name) => int_from_raw(name, &raw),
            _ => raw,
        };
        write!(out, "{value}").unwrap();
    }
    writeln!(out, ");").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    // ToRaw plus a matching release for whatever it allocated.
    writeln!(out, "    internal {c_ty} ToRaw()").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        var raw = new {c_ty}();").unwrap();
    for field in &item.fields {
        let raw = field.name.to_snake_case();
        let name = pascal(&field.name);
        match field.ty.unwrap_optional() {
            TypeRef::Bool => {
                writeln!(out, "        raw.{raw} = (byte)({name} ? 1 : 0);").unwrap();
            }
            TypeRef::String | TypeRef::CString => {
                writeln!(
                    out,
                    "        raw.{raw} = Marshal.StringToCoTaskMemUTF8({name});"
                )
                .unwrap();
            }
            TypeRef::Struct { .. } => {
                writeln!(out, "        raw.{raw} = {name}.ToRaw();").unwrap();
            }
            TypeRef::Callback { params } => {
                let delegate = struct_callback_delegate(ctx, item, field, params, prefix);
                let user_data = codegen_shared::naming::c_user_data_param(&field.name);
                writeln!(out, "        if ({name} is {{ }} {raw}Body)").unwrap();
                writeln!(out, "        {{").unwrap();
                writeln!(
                    out,
                    "            var {raw}Native = CallbackKeeper.Retain<{delegate}>({});",
                    trampoline_lambda(params, &format!("{raw}Body"))
                )
                .unwrap();
                writeln!(
                    out,
                    "            raw.{raw} = Marshal.GetFunctionPointerForDelegate({raw}Native);"
                )
                .unwrap();
                writeln!(out, "            raw.{user_data} = IntPtr.Zero;").unwrap();
                writeln!(out, "        }}").unwrap();
            }
            TypeRef::Int { name: int_name } if int_needs_conv(int_name) => {
                writeln!(out, "        raw.{raw} = {};", int_to_raw(int_name, &name)).unwrap();
            }
            _ => {
                writeln!(out, "        raw.{raw} = {name};").unwrap();
            }
        }
    }
    writeln!(out, "        return raw;").unwrap();
    writeln!(out, "    }}").unwrap();

    if struct_has_owned_fields(item) {
        // The matching C free lives with the struct definition so every file
        // that returns this struct shares one extern declaration.
        let free = c_free_symbol(prefix, &item.name);
        ctx.externs
            .insert(format!("internal static extern void {free}(ref {c_ty} value);"));
        writeln!(out).unwrap();
        writeln!(out, "    internal static void ReleaseRaw(ref {c_ty} raw)").unwrap();
        writeln!(out, "    {{").unwrap();
        for field in &item.fields {
            if matches!(
                field.ty.unwrap_optional(),
                TypeRef::String | TypeRef::CString
            ) {
                let raw = field.name.to_snake_case();
                writeln!(out, "        Marshal.FreeCoTaskMem(raw.{raw});").unwrap();
                writeln!(out, "        raw.{raw} = IntPtr.Zero;").unwrap();
            }
        }
        writeln!(out, "    }}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_event(out: &mut String, ctx: &mut FileCtx, group: &EventGroup, prefix: &str) {
    let raw_ty = c_type_name(prefix, &group.name);
    let union_variants: Vec<_> = group
        .variants
        .iter()
        .filter(|variant| !variant.fields.is_empty())
        .collect();

    // Raw C mirror: tag, common fields, then a union of the variant payloads.
    writeln!(out, "[StructLayout(LayoutKind.Sequential)]").unwrap();
    writeln!(out, "internal struct {raw_ty}").unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(out, "    internal int type;").unwrap();
    for field in &group.common {
        writeln!(
            out,
            "    internal {} {};",
            cs_raw_field_type(field.ty.unwrap_optional(), prefix),
            field.name.to_snake_case()
        )
        .unwrap();
    }
    if !union_variants.is_empty() {
        writeln!(out, "    internal DataUnion data;").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    [StructLayout(LayoutKind.Explicit)]").unwrap();
        writeln!(out, "    internal struct DataUnion").unwrap();
        writeln!(out, "    {{").unwrap();
        for variant in &union_variants {
            writeln!(
                out,
                "        [FieldOffset(0)] internal {}Data {};",
                pascal(&variant.discriminant),
                codegen_shared::naming::c_event_variant_field(&variant.discriminant)
            )
            .unwrap();
        }
        writeln!(out, "    }}").unwrap();
        for variant in &union_variants {
            writeln!(out).unwrap();
            writeln!(out, "    [StructLayout(LayoutKind.Sequential)]").unwrap();
            writeln!(out, "    internal struct {}Data", pascal(&variant.discriminant)).unwrap();
            writeln!(out, "    {{").unwrap();
            for field in &variant.fields {
                writeln!(
                    out,
                    "        internal {} {};",
                    cs_raw_field_type(field.ty.unwrap_optional(), prefix),
                    field.name.to_snake_case()
                )
                .unwrap();
            }
            writeln!(out, "    }}").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // The delegate every listener registration for this group goes through.
    ctx.delegates.insert(
        format!("{}NativeCallback", group.name),
        format!(
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]\ninternal delegate void {}NativeCallback(IntPtr evt, IntPtr userData);",
            group.name
        ),
    );

    // Public form: one record per concrete event.
    writeln!(out, "/// <summary>One {}, in its concrete form.</summary>", group.name).unwrap();
    writeln!(out, "public abstract record {}", group.name).unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(out, "    private {}() {{ }}", group.name).unwrap();
    writeln!(out).unwrap();
    for variant in &group.variants {
        let fields: Vec<String> = group
            .common
            .iter()
            .chain(variant.fields.iter())
            .map(|field| {
                format!(
                    "{} {}",
                    cs_event_field_type(&field.ty),
                    pascal(&field.name)
                )
            })
            .collect();
        if fields.is_empty() {
            writeln!(
                out,
                "    public sealed record {} : {};",
                pascal(&variant.discriminant),
                group.name
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "    public sealed record {}({}) : {};",
                pascal(&variant.discriminant),
                fields.join(", "),
                group.name
            )
            .unwrap();
        }
    }
    writeln!(out).unwrap();

    writeln!(
        out,
        "    internal static {}? FromRaw(in {raw_ty} raw)",
        group.name
    )
    .unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        switch (raw.type)").unwrap();
    writeln!(out, "        {{").unwrap();
    for (index, variant) in group.variants.iter().enumerate() {
        let payload = codegen_shared::naming::c_event_variant_field(&variant.discriminant);
        let mut values: Vec<String> = Vec::new();
        for field in &group.common {
            values.push(cs_event_field_expr(
                &field.ty,
                &format!("raw.{}", field.name.to_snake_case()),
            ));
        }
        for field in &variant.fields {
            values.push(cs_event_field_expr(
                &field.ty,
                &format!("raw.data.{payload}.{}", field.name.to_snake_case()),
            ));
        }
        writeln!(
            out,
            "            case {index}: return new {}({});",
            pascal(&variant.discriminant),
            values.join(", ")
        )
        .unwrap();
    }
    writeln!(out, "            default: return null;").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Event payload handles are borrowed: the C side releases them when the
/// callback returns, so wrappers are built with `ownsHandle: false`.
fn cs_event_field_expr(ty: &TypeRef, access: &str) -> String {
    match ty.unwrap_optional() {
        TypeRef::String | TypeRef::CString => format!("Marshal.PtrToStringUTF8({access})"),
        TypeRef::Struct { name, .. } => format!("{name}.FromRaw(in {access})"),
        TypeRef::Object { name, .. } => format!("new {name}({access}, ownsHandle: false)"),
        TypeRef::Int { name } if int_needs_conv(name) => int_from_raw(name, access),
        _ => access.to_string(),
    }
}

fn cs_event_field_type(ty: &TypeRef) -> String {
    match ty.unwrap_optional() {
        TypeRef::Object { name, .. } => name.clone(),
        TypeRef::String | TypeRef::CString => "string?".to_string(),
        other => cs_public_type(other),
    }
}

fn generate_list_struct(out: &mut String, ctx: &mut FileCtx, class_name: &str, prefix: &str) {
    let list_ty = c_list_type_name(prefix, class_name);
    let field = c_list_field(class_name);
    writeln!(out, "[StructLayout(LayoutKind.Sequential)]").unwrap();
    writeln!(out, "internal struct {list_ty}").unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(out, "    internal IntPtr {field};").unwrap();
    writeln!(out, "    internal CLong count;").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    // The release extern lives with the list struct so every file that returns
    // this list shares one declaration.
    let release = c_list_release_symbol(prefix, class_name);
    ctx.externs.insert(format!(
        "internal static extern void {release}(ref {list_ty} list);"
    ));
}

// ---------------------------------------------------------------------------
// Classes
// ---------------------------------------------------------------------------

fn generate_handle_class(
    out: &mut String,
    ctx: &mut FileCtx,
    api: &Api,
    header: &Header,
    class: &Class,
    prefix: &str,
) {
    let free_symbol = c_free_symbol(prefix, &class.name);
    ctx.externs
        .insert(format!("internal static extern void {free_symbol}(ulong handle);"));

    writeln!(out, "/// <summary>Owned handle to a native {}.</summary>", class.name).unwrap();
    writeln!(out, "public sealed partial class {} : IDisposable", class.name).unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(out, "    public ulong NativeHandle {{ get; private set; }}").unwrap();
    writeln!(out, "    private readonly bool _ownsHandle;").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "    public {}(ulong nativeHandle, bool ownsHandle = true)",
        class.name
    )
    .unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        NativeHandle = nativeHandle;").unwrap();
    writeln!(out, "        _ownsHandle = ownsHandle;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    ~{}() => ReleaseHandle();", class.name).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    public void Dispose()").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        ReleaseHandle();").unwrap();
    writeln!(out, "        GC.SuppressFinalize(this);").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    private void ReleaseHandle()").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        if (_ownsHandle && NativeHandle != 0)").unwrap();
    writeln!(out, "        {{").unwrap();
    writeln!(out, "            Interop.{free_symbol}(NativeHandle);").unwrap();
    writeln!(out, "            NativeHandle = 0;").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    for ctor in &class.constructors {
        generate_constructor(out, ctx, api, class, ctor, prefix);
    }

    for method in &class.methods {
        generate_method(out, ctx, api, header, class, method, prefix);
    }

    if class.native_object {
        let symbol = c_native_object_symbol(prefix, &class.name);
        ctx.externs
            .insert(format!("internal static extern IntPtr {symbol}(ulong handle);"));
        writeln!(
            out,
            "    /// <summary>Platform-specific native object behind this handle.</summary>"
        )
        .unwrap();
        writeln!(out, "    public IntPtr NativeObject => Interop.{symbol}(NativeHandle);").unwrap();
        writeln!(out).unwrap();
    }

    generate_listener(out, ctx, api, class, prefix);

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_singleton_class(
    out: &mut String,
    ctx: &mut FileCtx,
    api: &Api,
    header: &Header,
    class: &Class,
    prefix: &str,
) {
    writeln!(out, "public sealed partial class {}", class.name).unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(
        out,
        "    /// <summary>The shared instance backed by the native singleton.</summary>"
    )
    .unwrap();
    writeln!(
        out,
        "    public static {} Shared {{ get; }} = new {}();",
        class.name, class.name
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    private {}() {{ }}", class.name).unwrap();
    writeln!(out).unwrap();

    for method in &class.methods {
        generate_method(out, ctx, api, header, class, method, prefix);
    }

    generate_listener(out, ctx, api, class, prefix);

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_constructor(
    out: &mut String,
    ctx: &mut FileCtx,
    api: &Api,
    class: &Class,
    ctor: &Constructor,
    prefix: &str,
) {
    let label = match constructor_suffix(class, ctor) {
        Some(suffix) => format!("Create{}", pascal(&suffix)),
        None => "Create".to_string(),
    };
    let symbol = c_constructor_symbol(prefix, class, ctor);
    let owner = format!("{}.{label}", class.name);
    let delegates = ctx_delegates(ctx, &owner, &ctor.params, prefix);
    ctx.externs.insert(extern_decl(
        &symbol,
        &TypeRef::Object {
            name: class.name.clone(),
            qualified_name: class.qualified_name.clone(),
            shared: false,
        },
        None,
        &ctor.params,
        delegates,
        prefix,
    ));

    writeln!(
        out,
        "    /// <summary>Creates a new {}; returns null if the native side failed.</summary>",
        class.name
    )
    .unwrap();
    writeln!(
        out,
        "    public static {}? {label}({})",
        class.name,
        cs_params(&ctor.params)
    )
    .unwrap();
    writeln!(out, "    {{").unwrap();
    let bindings = render_param_bindings(out, ctx, api, &owner, &ctor.params, prefix, "        ");
    writeln!(
        out,
        "        var handle = Interop.{symbol}({});",
        call_args(&ctor.params, None)
    )
    .unwrap();
    render_param_cleanup(out, api, &ctor.params, &bindings, "        ");
    writeln!(out, "        return handle == 0 ? null : new {}(handle);", class.name).unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

fn generate_method(
    out: &mut String,
    ctx: &mut FileCtx,
    api: &Api,
    header: &Header,
    class: &Class,
    method: &Method,
    prefix: &str,
) {
    let instance = class.is_instance() && !method.is_static;
    let name = cs_method_name(class, method);
    let symbol = c_method_symbol(prefix, class, method);
    let owner = format!("{}.{name}", class.name);

    let delegates = ctx_delegates(ctx, &owner, &method.params, prefix);
    ctx.externs.insert(extern_decl(
        &symbol,
        &method.return_type,
        instance.then_some("ulong self"),
        &method.params,
        delegates,
        prefix,
    ));

    let return_type = cs_return_type(&method.return_type);
    // A parameterless const getter reads better as a property, unless its name
    // would collide with the enclosing type.
    let accessor = is_binding_accessor(class, method) && method.params.is_empty();
    let property = accessor && name != class.name;

    if property {
        writeln!(out, "    public {return_type} {name}").unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        get").unwrap();
        writeln!(out, "        {{").unwrap();
    } else {
        let keyword = if class.is_instance() && method.is_static {
            "public static"
        } else {
            "public"
        };
        writeln!(
            out,
            "    {keyword} {return_type} {name}({})",
            cs_params(&method.params)
        )
        .unwrap();
        writeln!(out, "    {{").unwrap();
    }
    let indent = if property { "            " } else { "        " };

    let bindings = render_param_bindings(out, ctx, api, &owner, &method.params, prefix, indent);
    let receiver = instance.then(|| "NativeHandle".to_string());
    let args = call_args(&method.params, receiver);

    if matches!(method.return_type, TypeRef::Void) {
        writeln!(out, "{indent}Interop.{symbol}({args});").unwrap();
        render_param_cleanup(out, api, &method.params, &bindings, indent);
    } else {
        writeln!(out, "{indent}var rawResult = Interop.{symbol}({args});").unwrap();
        render_param_cleanup(out, api, &method.params, &bindings, indent);
        render_return(out, api, header, &method.return_type, prefix, indent);
    }

    if property {
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
    } else {
        writeln!(out, "    }}").unwrap();
    }
    writeln!(out).unwrap();
}

fn generate_listener(out: &mut String, ctx: &mut FileCtx, api: &Api, class: &Class, prefix: &str) {
    let Some(group) = emitted_group(api, class) else {
        return;
    };
    let raw_ty = c_type_name(prefix, &group.name);
    let delegate = format!("{}NativeCallback", group.name);
    let add_symbol = c_add_listener_symbol(prefix, &class.name);
    let remove_symbol = c_remove_listener_symbol(prefix, &class.name);
    let instance = class.is_instance();
    let self_param = if instance { "ulong self, " } else { "" };
    let self_arg = if instance { "NativeHandle, " } else { "" };

    ctx.externs.insert(format!(
        "internal static extern ulong {add_symbol}({self_param}{delegate} callback, IntPtr userData);"
    ));
    ctx.externs.insert(format!(
        "[return: MarshalAs(UnmanagedType.I1)]\n    internal static extern bool {remove_symbol}({self_param}ulong listenerId);"
    ));

    writeln!(
        out,
        "    /// <summary>Registers <paramref name=\"callback\"/> for every {} this {} emits.</summary>",
        group.name, class.name
    )
    .unwrap();
    writeln!(out, "    /// <remarks>").unwrap();
    writeln!(
        out,
        "    /// The delegate is retained for good: the C ABI keeps the context"
    )
    .unwrap();
    writeln!(
        out,
        "    /// pointer but offers no hook to release it, so removing the listener"
    )
    .unwrap();
    writeln!(out, "    /// stops the calls without freeing the delegate.").unwrap();
    writeln!(out, "    /// </remarks>").unwrap();
    writeln!(
        out,
        "    public ulong AddListener(Action<{}> callback)",
        group.name
    )
    .unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(
        out,
        "        var native = CallbackKeeper.Retain<{delegate}>((evt, userData) =>"
    )
    .unwrap();
    writeln!(out, "        {{").unwrap();
    writeln!(out, "            if (evt == IntPtr.Zero)").unwrap();
    writeln!(out, "            {{").unwrap();
    writeln!(out, "                return;").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(
        out,
        "            var value = {}.FromRaw(Marshal.PtrToStructure<{raw_ty}>(evt));",
        group.name
    )
    .unwrap();
    writeln!(out, "            if (value is not null)").unwrap();
    writeln!(out, "            {{").unwrap();
    writeln!(out, "                callback(value);").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }});").unwrap();
    writeln!(
        out,
        "        return Interop.{add_symbol}({self_arg}native, IntPtr.Zero);"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "    /// <summary>Unregisters a listener. Returns false if unknown.</summary>"
    )
    .unwrap();
    writeln!(out, "    public bool RemoveListener(ulong listenerId)").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(
        out,
        "        return Interop.{remove_symbol}({self_arg}listenerId);"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

fn emitted_group<'a>(api: &'a Api, class: &Class) -> Option<&'a EventGroup> {
    let event = class.event.as_ref()?;
    api.headers
        .iter()
        .flat_map(|header| header.events.iter())
        .find(|group| &group.name == event)
}

// ---------------------------------------------------------------------------
// Callback delegates
// ---------------------------------------------------------------------------

/// Registers (and names) the delegate types for every callback parameter in
/// `params`, returning name-by-parameter for the extern declaration.
fn ctx_delegates(
    ctx: &mut FileCtx,
    owner: &str,
    params: &[Param],
    prefix: &str,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for param in params {
        if let TypeRef::Callback { params: args } = param.ty.unwrap_optional() {
            let name = format!(
                "{}{}NativeCallback",
                owner.replace('.', ""),
                pascal(&param.name)
            );
            register_delegate(ctx, &name, args, prefix);
            map.insert(param.name.clone(), name);
        }
    }
    map
}

fn struct_callback_delegate(
    ctx: &mut FileCtx,
    item: &Struct,
    field: &codegen_shared::ir::Field,
    args: &[TypeRef],
    prefix: &str,
) -> String {
    let name = format!("{}{}NativeCallback", item.name, pascal(&field.name));
    register_delegate(ctx, &name, args, prefix);
    name
}

fn register_delegate(ctx: &mut FileCtx, name: &str, args: &[TypeRef], prefix: &str) {
    let mut params: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(index, ty)| format!("{} arg{index}", cs_callback_c_type(ty, prefix)))
        .collect();
    params.push("IntPtr userData".to_string());
    ctx.delegates.insert(
        name.to_string(),
        format!(
            "[UnmanagedFunctionPointer(CallingConvention.Cdecl)]\ninternal delegate void {name}({});",
            params.join(", ")
        ),
    );
}

/// How a callback argument arrives from C.
fn cs_callback_c_type(ty: &TypeRef, prefix: &str) -> String {
    match ty {
        TypeRef::Bool => "[MarshalAs(UnmanagedType.I1)] bool".to_string(),
        TypeRef::String | TypeRef::CString => "IntPtr".to_string(),
        TypeRef::Struct { name, .. } => c_type_name(prefix, name),
        TypeRef::Object { .. } => "ulong".to_string(),
        TypeRef::Alias { underlying, .. } => cs_callback_c_type(underlying, prefix),
        TypeRef::Int { name } => cs_raw_int(name).to_string(),
        TypeRef::Float { name } => cs_float(name).to_string(),
        TypeRef::Enum { name, .. } => name.clone(),
        _ => "IntPtr".to_string(),
    }
}

/// The lambda bridging C-level callback arguments to the public `Action`.
fn trampoline_lambda(args: &[TypeRef], body: &str) -> String {
    let mut params: Vec<String> = (0..args.len()).map(|index| format!("arg{index}")).collect();
    params.push("userData".to_string());
    let converted: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(index, ty)| cs_callback_arg_expr(ty, &format!("arg{index}")))
        .collect();
    format!(
        "({}) => {body}({})",
        params.join(", "),
        converted.join(", ")
    )
}

fn cs_callback_arg_expr(ty: &TypeRef, access: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::CString => format!("Marshal.PtrToStringUTF8({access})"),
        TypeRef::Struct { name, .. } => format!("{name}.FromRaw(in {access})"),
        TypeRef::Object { name, .. } => {
            format!("{access} == 0 ? null : new {name}({access}, ownsHandle: false)")
        }
        TypeRef::Alias { underlying, .. } => cs_callback_arg_expr(underlying, access),
        TypeRef::Int { name } if int_needs_conv(name) => int_from_raw(name, access),
        _ => access.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Externs
// ---------------------------------------------------------------------------

fn extern_decl(
    symbol: &str,
    return_type: &TypeRef,
    receiver: Option<&str>,
    params: &[Param],
    delegates: BTreeMap<String, String>,
    prefix: &str,
) -> String {
    let mut parts: Vec<String> = receiver.map(str::to_string).into_iter().collect();
    for param in params {
        let name = cs_ident(&param.name);
        match param.ty.unwrap_optional() {
            TypeRef::String | TypeRef::CString => parts.push(format!(
                "[MarshalAs(UnmanagedType.LPUTF8Str)] string? {name}"
            )),
            TypeRef::Bool => parts.push(format!("[MarshalAs(UnmanagedType.I1)] bool {name}")),
            TypeRef::Int { name: int } => parts.push(format!("{} {name}", cs_raw_int(int))),
            TypeRef::Float { name: float } => {
                parts.push(format!("{} {name}", cs_float(float)))
            }
            TypeRef::Enum { name: ty, .. } => parts.push(format!("{ty} {name}")),
            TypeRef::Struct { name: ty, .. } => {
                if matches!(param.ty, TypeRef::Optional { .. }) {
                    parts.push(format!("IntPtr {name}"));
                } else {
                    parts.push(format!("{} {name}", c_type_name(prefix, ty)));
                }
            }
            TypeRef::Object { .. } => parts.push(format!("ulong {name}")),
            TypeRef::Alias { underlying, .. } => {
                parts.push(format!("{} {name}", cs_raw_type_of(underlying, prefix)))
            }
            TypeRef::Vector { .. } => parts.push(format!("native_string_list_t {name}")),
            TypeRef::Map { .. } => parts.push(format!("native_string_map_t {name}")),
            TypeRef::Callback { .. } => {
                let delegate = delegates
                    .get(&param.name)
                    .cloned()
                    .unwrap_or_else(|| "IntPtr".to_string());
                let optional = if matches!(param.ty, TypeRef::Optional { .. }) {
                    "?"
                } else {
                    ""
                };
                parts.push(format!("{delegate}{optional} {name}"));
                parts.push(format!(
                    "IntPtr {}",
                    codegen_shared::naming::c_user_data_param(&param.name)
                ));
            }
            TypeRef::RawPointer => parts.push(format!("IntPtr {name}")),
            _ => parts.push(format!("IntPtr {name}")),
        }
    }

    let (attr, ret) = extern_return(return_type, prefix);
    format!(
        "{attr}internal static extern {ret} {symbol}({});",
        parts.join(", ")
    )
}

fn extern_return(ty: &TypeRef, prefix: &str) -> (String, String) {
    match ty {
        TypeRef::Void => (String::new(), "void".to_string()),
        TypeRef::Bool => (
            "[return: MarshalAs(UnmanagedType.I1)]\n    ".to_string(),
            "bool".to_string(),
        ),
        TypeRef::String | TypeRef::CString => (String::new(), "IntPtr".to_string()),
        TypeRef::Int { name } => (String::new(), cs_raw_int(name).to_string()),
        TypeRef::Float { name } => (String::new(), cs_float(name).to_string()),
        TypeRef::Enum { name, .. } => (String::new(), name.clone()),
        TypeRef::Struct { name, .. } => (String::new(), c_type_name(prefix, name)),
        TypeRef::Object { .. } => (String::new(), "ulong".to_string()),
        TypeRef::Alias { underlying, .. } => extern_return(underlying, prefix),
        TypeRef::Vector { element } => match element.as_ref() {
            TypeRef::Object { name, .. } => {
                (String::new(), c_list_type_name(prefix, name))
            }
            _ => (String::new(), "native_string_list_t".to_string()),
        },
        TypeRef::Map { .. } => (String::new(), "native_string_map_t".to_string()),
        TypeRef::Optional { inner } => extern_return(inner, prefix),
        TypeRef::RawPointer => (String::new(), "IntPtr".to_string()),
        _ => (String::new(), "IntPtr".to_string()),
    }
}

fn cs_raw_type_of(ty: &TypeRef, prefix: &str) -> String {
    match ty {
        TypeRef::Int { name } => cs_raw_int(name).to_string(),
        TypeRef::Float { name } => cs_float(name).to_string(),
        other => cs_raw_field_type(other, prefix),
    }
}

// ---------------------------------------------------------------------------
// Parameters and returns
// ---------------------------------------------------------------------------

/// What a bound parameter left behind that the call and cleanup lines use.
enum Binding {
    None,
    StructRaw,
    StructPtr,
    StringList,
    StringMap,
    Callback,
}

fn cs_params(params: &[Param]) -> String {
    params
        .iter()
        .map(|param| format!("{} {}", cs_param_type(&param.ty), cs_ident(&param.name)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cs_param_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::CString => "string".to_string(),
        TypeRef::Object { name, shared: true, .. } => format!("{name}?"),
        TypeRef::Object { name, .. } => name.clone(),
        TypeRef::Vector { .. } => "IReadOnlyList<string>".to_string(),
        TypeRef::Map { .. } => "IReadOnlyDictionary<string, string>".to_string(),
        TypeRef::Callback { params } => cs_action_type(params),
        TypeRef::Optional { inner } => match inner.as_ref() {
            TypeRef::String | TypeRef::CString => "string?".to_string(),
            TypeRef::Callback { params } => format!("{}?", cs_action_type(params)),
            other => {
                let base = cs_param_type(other);
                if base.ends_with('?') {
                    base
                } else {
                    format!("{base}?")
                }
            }
        },
        other => cs_public_type(other),
    }
}

fn cs_action_type(params: &[TypeRef]) -> String {
    if params.is_empty() {
        "Action".to_string()
    } else {
        format!(
            "Action<{}>",
            params
                .iter()
                .map(cs_public_type)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Locals the call needs; returns what each parameter left behind.
fn render_param_bindings(
    out: &mut String,
    ctx: &mut FileCtx,
    api: &Api,
    owner: &str,
    params: &[Param],
    prefix: &str,
    indent: &str,
) -> Vec<Binding> {
    let mut bindings = Vec::new();
    for param in params {
        let name = cs_ident(&param.name);
        let local = param.name.to_snake_case().to_upper_camel_case();
        let binding = match &param.ty {
            TypeRef::Struct { .. } => {
                writeln!(out, "{indent}var raw{local} = {name}.ToRaw();").unwrap();
                Binding::StructRaw
            }
            TypeRef::Vector { .. } => {
                writeln!(out, "{indent}var items{local} = Interop.AllocUtf8Array({name});").unwrap();
                writeln!(
                    out,
                    "{indent}var block{local} = Interop.AllocPointerArray(items{local});"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var list{local} = new native_string_list_t {{ items = block{local}, count = new CLong(items{local}.Length) }};"
                )
                .unwrap();
                Binding::StringList
            }
            TypeRef::Map { .. } => {
                writeln!(
                    out,
                    "{indent}var keyItems{local} = Interop.AllocUtf8Array(System.Linq.Enumerable.ToArray({name}.Keys));"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var valueItems{local} = Interop.AllocUtf8Array(System.Linq.Enumerable.ToArray({name}.Values));"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var keyBlock{local} = Interop.AllocPointerArray(keyItems{local});"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var valueBlock{local} = Interop.AllocPointerArray(valueItems{local});"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var map{local} = new native_string_map_t {{ keys = keyBlock{local}, values = valueBlock{local}, count = new CLong(keyItems{local}.Length) }};"
                )
                .unwrap();
                Binding::StringMap
            }
            TypeRef::Callback { params: args } => {
                let delegate = ctx_delegate_name(ctx, owner, &param.name, args, prefix);
                writeln!(
                    out,
                    "{indent}var native{local} = CallbackKeeper.Retain<{delegate}>({});",
                    trampoline_lambda(args, &name)
                )
                .unwrap();
                Binding::Callback
            }
            TypeRef::Optional { inner } => match inner.as_ref() {
                TypeRef::Struct { name: type_name, .. } => {
                    let c_ty = c_type_name(prefix, type_name);
                    writeln!(out, "{indent}var ptr{local} = IntPtr.Zero;").unwrap();
                    writeln!(out, "{indent}if ({name} is {{ }} value{local})").unwrap();
                    writeln!(out, "{indent}{{").unwrap();
                    writeln!(out, "{indent}    var raw{local} = value{local}.ToRaw();").unwrap();
                    writeln!(
                        out,
                        "{indent}    ptr{local} = Marshal.AllocHGlobal(Marshal.SizeOf<{c_ty}>());"
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "{indent}    Marshal.StructureToPtr(raw{local}, ptr{local}, false);"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                    Binding::StructPtr
                }
                TypeRef::Callback { params: args } => {
                    let delegate = ctx_delegate_name(ctx, owner, &param.name, args, prefix);
                    writeln!(out, "{indent}{delegate}? native{local} = null;").unwrap();
                    writeln!(out, "{indent}if ({name} is {{ }} body{local})").unwrap();
                    writeln!(out, "{indent}{{").unwrap();
                    writeln!(
                        out,
                        "{indent}    native{local} = CallbackKeeper.Retain<{delegate}>({});",
                        trampoline_lambda(args, &format!("body{local}"))
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                    Binding::Callback
                }
                _ => Binding::None,
            },
            _ => Binding::None,
        };
        bindings.push(binding);
    }
    let _ = api;
    bindings
}

fn ctx_delegate_name(
    ctx: &mut FileCtx,
    owner: &str,
    param: &str,
    args: &[TypeRef],
    prefix: &str,
) -> String {
    let name = format!("{}{}NativeCallback", owner.replace('.', ""), pascal(param));
    register_delegate(ctx, &name, args, prefix);
    name
}

fn render_param_cleanup(
    out: &mut String,
    api: &Api,
    params: &[Param],
    bindings: &[Binding],
    indent: &str,
) {
    for (param, binding) in params.iter().zip(bindings.iter()) {
        let local = param.name.to_snake_case().to_upper_camel_case();
        match binding {
            Binding::StructRaw => {
                if let TypeRef::Struct { name, .. } = param.ty.unwrap_optional() {
                    if struct_owns_memory_api(api, name) {
                        writeln!(out, "{indent}{name}.ReleaseRaw(ref raw{local});").unwrap();
                    }
                }
            }
            Binding::StructPtr => {
                writeln!(out, "{indent}if (ptr{local} != IntPtr.Zero)").unwrap();
                writeln!(out, "{indent}{{").unwrap();
                if let TypeRef::Struct { name, .. } = param.ty.unwrap_optional() {
                    if struct_owns_memory_api(api, name) {
                        let c_ty_var = format!("owned{local}");
                        writeln!(
                            out,
                            "{indent}    var {c_ty_var} = Marshal.PtrToStructure<{}>(ptr{local});",
                            struct_c_type_from_api(api, name)
                        )
                        .unwrap();
                        writeln!(out, "{indent}    {name}.ReleaseRaw(ref {c_ty_var});").unwrap();
                    }
                }
                writeln!(out, "{indent}    Marshal.FreeHGlobal(ptr{local});").unwrap();
                writeln!(out, "{indent}}}").unwrap();
            }
            Binding::StringList => {
                writeln!(out, "{indent}Interop.FreeUtf8Array(items{local});").unwrap();
                writeln!(out, "{indent}Marshal.FreeHGlobal(block{local});").unwrap();
            }
            Binding::StringMap => {
                writeln!(out, "{indent}Interop.FreeUtf8Array(keyItems{local});").unwrap();
                writeln!(out, "{indent}Interop.FreeUtf8Array(valueItems{local});").unwrap();
                writeln!(out, "{indent}Marshal.FreeHGlobal(keyBlock{local});").unwrap();
                writeln!(out, "{indent}Marshal.FreeHGlobal(valueBlock{local});").unwrap();
            }
            Binding::Callback | Binding::None => {}
        }
    }
}

fn struct_owns_memory_api(api: &Api, name: &str) -> bool {
    api.headers
        .iter()
        .flat_map(|header| header.structs.iter())
        .any(|item| item.name == name && struct_has_owned_fields(item))
}

fn struct_owns_memory(api: &Api, header: &Header, name: &str) -> bool {
    header
        .structs
        .iter()
        .chain(api.headers.iter().flat_map(|h| h.structs.iter()))
        .any(|item| item.name == name && struct_has_owned_fields(item))
}

fn struct_c_type_from_api(api: &Api, name: &str) -> String {
    let _ = api;
    c_type_name("native_", name)
}

fn call_args(params: &[Param], receiver: Option<String>) -> String {
    let mut args: Vec<String> = receiver.into_iter().collect();
    for param in params {
        let name = cs_ident(&param.name);
        let local = param.name.to_snake_case().to_upper_camel_case();
        match &param.ty {
            TypeRef::Object { shared: true, .. } => {
                args.push(format!("{name}?.NativeHandle ?? 0"))
            }
            TypeRef::Object { .. } => args.push(format!("{name}.NativeHandle")),
            TypeRef::Struct { .. } => args.push(format!("raw{local}")),
            TypeRef::Vector { .. } => args.push(format!("list{local}")),
            TypeRef::Map { .. } => args.push(format!("map{local}")),
            TypeRef::Callback { .. } => {
                args.push(format!("native{local}"));
                args.push("IntPtr.Zero".to_string());
            }
            TypeRef::Int { name: int } if int_needs_conv(int) => {
                args.push(int_to_raw(int, &name))
            }
            TypeRef::Alias { underlying, .. } => match underlying.as_ref() {
                TypeRef::Int { name: int } if int_needs_conv(int) => {
                    args.push(int_to_raw(int, &name))
                }
                _ => args.push(name),
            },
            TypeRef::Optional { inner } => match inner.as_ref() {
                TypeRef::Object { .. } => args.push(format!("{name}?.NativeHandle ?? 0")),
                TypeRef::Struct { .. } => args.push(format!("ptr{local}")),
                TypeRef::Callback { .. } => {
                    args.push(format!("native{local}"));
                    args.push("IntPtr.Zero".to_string());
                }
                _ => args.push(name),
            },
            _ => args.push(name),
        }
    }
    args.join(", ")
}

/// Converts `rawResult` into the public return value.
fn render_return(
    out: &mut String,
    api: &Api,
    header: &Header,
    ty: &TypeRef,
    prefix: &str,
    indent: &str,
) {
    match ty {
        TypeRef::Void => {}
        TypeRef::String | TypeRef::CString => {
            writeln!(out, "{indent}return Interop.ConsumeString(rawResult);").unwrap();
        }
        TypeRef::Struct { name, .. } => {
            if struct_owns_memory(api, header, name) {
                writeln!(out, "{indent}var result = {name}.FromRaw(in rawResult);").unwrap();
                writeln!(
                    out,
                    "{indent}Interop.{}(ref rawResult);",
                    c_free_symbol(prefix, name)
                )
                .unwrap();
                writeln!(out, "{indent}return result;").unwrap();
            } else {
                writeln!(out, "{indent}return {name}.FromRaw(in rawResult);").unwrap();
            }
        }
        TypeRef::Enum { .. } => {
            writeln!(out, "{indent}return rawResult;").unwrap();
        }
        TypeRef::Object { name, .. } => {
            writeln!(
                out,
                "{indent}return rawResult == 0 ? null : new {name}(rawResult);"
            )
            .unwrap();
        }
        TypeRef::Vector { element } => match element.as_ref() {
            TypeRef::Object { name, .. } => {
                let field = c_list_field(name);
                writeln!(
                    out,
                    "{indent}var count = rawResult.{field} == IntPtr.Zero ? 0 : checked((int)rawResult.count.Value);"
                )
                .unwrap();
                writeln!(out, "{indent}var items = new {name}[count];").unwrap();
                writeln!(out, "{indent}for (var i = 0; i < count; i++)").unwrap();
                writeln!(out, "{indent}{{").unwrap();
                writeln!(
                    out,
                    "{indent}    items[i] = new {name}((ulong)Marshal.ReadInt64(rawResult.{field}, i * 8));"
                )
                .unwrap();
                writeln!(out, "{indent}}}").unwrap();
                writeln!(
                    out,
                    "{indent}// The handles now belong to `items`; free just the array."
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}Interop.{}(ref rawResult);",
                    c_list_release_symbol(prefix, name)
                )
                .unwrap();
                writeln!(out, "{indent}return items;").unwrap();
            }
            _ => {
                writeln!(
                    out,
                    "{indent}return Interop.ConsumeStringList(ref rawResult);"
                )
                .unwrap();
            }
        },
        TypeRef::Map { .. } => {
            writeln!(out, "{indent}return Interop.ConsumeStringMap(ref rawResult);").unwrap();
        }
        TypeRef::Optional { inner } => render_return(out, api, header, inner, prefix, indent),
        TypeRef::Int { name } if int_needs_conv(name) => {
            writeln!(out, "{indent}return {};", int_from_raw(name, "rawResult")).unwrap();
        }
        _ => {
            writeln!(out, "{indent}return rawResult;").unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Types and names
// ---------------------------------------------------------------------------

fn cs_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Void => "void".to_string(),
        TypeRef::String | TypeRef::CString => "string?".to_string(),
        TypeRef::Object { name, .. } => format!("{name}?"),
        TypeRef::Vector { element } => match element.as_ref() {
            TypeRef::Object { name, .. } => format!("{name}[]"),
            _ => "string[]".to_string(),
        },
        TypeRef::Map { .. } => "Dictionary<string, string>".to_string(),
        TypeRef::Optional { inner } => {
            let base = cs_return_type(inner);
            if base.ends_with('?') {
                base
            } else {
                format!("{base}?")
            }
        }
        other => cs_public_type(other),
    }
}

fn cs_public_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Void => "void".to_string(),
        TypeRef::Bool => "bool".to_string(),
        TypeRef::Int { name } => cs_public_int(name).to_string(),
        TypeRef::Float { name } => cs_float(name).to_string(),
        TypeRef::String | TypeRef::CString => "string?".to_string(),
        TypeRef::Alias { underlying, .. } => cs_public_type(underlying),
        TypeRef::Enum { name, .. } | TypeRef::Struct { name, .. } => name.clone(),
        TypeRef::Object { name, .. } => format!("{name}?"),
        TypeRef::Vector { element } => format!("{}[]", cs_public_type(element)),
        TypeRef::Map { .. } => "Dictionary<string, string>".to_string(),
        TypeRef::Optional { inner } => {
            let base = cs_public_type(inner);
            if base.ends_with('?') {
                base
            } else {
                format!("{base}?")
            }
        }
        TypeRef::RawPointer => "IntPtr".to_string(),
        TypeRef::Callback { .. } | TypeRef::Unsupported { .. } => "void".to_string(),
    }
}

fn cs_struct_field_type(ty: &TypeRef) -> String {
    match ty.unwrap_optional() {
        TypeRef::Callback { params } => format!("{}?", cs_action_type(params)),
        TypeRef::String | TypeRef::CString => "string?".to_string(),
        other => cs_public_type(other),
    }
}

/// The blittable mirror of a C struct field.
fn cs_raw_field_type(ty: &TypeRef, prefix: &str) -> String {
    match ty {
        TypeRef::Bool => "byte".to_string(),
        TypeRef::Int { name } => cs_raw_int(name).to_string(),
        TypeRef::Float { name } => cs_float(name).to_string(),
        TypeRef::String | TypeRef::CString => "IntPtr".to_string(),
        TypeRef::Enum { name, .. } => name.clone(),
        TypeRef::Struct { name, .. } => c_type_name(prefix, name),
        TypeRef::Object { .. } => "ulong".to_string(),
        TypeRef::Alias { underlying, .. } => cs_raw_field_type(underlying, prefix),
        TypeRef::RawPointer => "IntPtr".to_string(),
        _ => "IntPtr".to_string(),
    }
}

fn cs_public_int(name: &str) -> &'static str {
    match name {
        "char" | "signed char" => "sbyte",
        "short" => "short",
        "int" => "int",
        "long" => "long",
        "long long" => "long",
        "unsigned char" => "byte",
        "unsigned short" => "ushort",
        "unsigned int" => "uint",
        "unsigned long" => "ulong",
        "unsigned long long" => "ulong",
        _ => "int",
    }
}

fn cs_raw_int(name: &str) -> &'static str {
    match name {
        "long" => "CLong",
        "unsigned long" => "CULong",
        other => cs_public_int(other),
    }
}

fn int_needs_conv(name: &str) -> bool {
    matches!(name, "long" | "unsigned long")
}

fn int_to_raw(name: &str, value: &str) -> String {
    match name {
        "long" => format!("new CLong(checked((nint){value}))"),
        _ => format!("new CULong(checked((nuint){value}))"),
    }
}

fn int_from_raw(name: &str, value: &str) -> String {
    match name {
        "long" => format!("(long){value}.Value"),
        _ => format!("(ulong){value}.Value"),
    }
}

fn cs_float(name: &str) -> &'static str {
    match name {
        "float" => "float",
        _ => "double",
    }
}

fn enum_member(name: &str) -> String {
    name.strip_prefix('k').unwrap_or(name).to_upper_camel_case()
}

fn pascal(name: &str) -> String {
    name.to_upper_camel_case()
}

const CSHARP_KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked",
    "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else",
    "enum", "event", "explicit", "extern", "false", "finally", "fixed", "float", "for",
    "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock",
    "long", "namespace", "new", "null", "object", "operator", "out", "override", "params",
    "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short",
    "sizeof", "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true",
    "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual",
    "void", "volatile", "while",
];

fn cs_ident(name: &str) -> String {
    let ident = name.to_snake_case().to_lower_camel_case();
    if CSHARP_KEYWORDS.contains(&ident.as_str()) {
        format!("@{ident}")
    } else {
        ident
    }
}

fn cs_method_name(class: &Class, method: &Method) -> String {
    let base = if is_binding_accessor(class, method) {
        method.binding_name()
    } else {
        &method.name
    };
    pascal(&with_overload_suffix(base.to_snake_case(), class, method))
}

fn with_overload_suffix(base: String, class: &Class, method: &Method) -> String {
    let overloaded = class
        .methods
        .iter()
        .filter(|other| other.name == method.name)
        .count()
        > 1;
    if !overloaded || method.params.is_empty() {
        return base;
    }
    format!(
        "{base}_with_{}",
        method
            .params
            .iter()
            .map(|param| param.name.to_snake_case())
            .collect::<Vec<_>>()
            .join("_and_")
    )
}

use heck::ToLowerCamelCase;
