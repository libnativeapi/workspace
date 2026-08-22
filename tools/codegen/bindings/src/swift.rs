use std::fmt::Write;
use std::path::Path;

use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};

use codegen_shared::naming::{
    c_add_listener_symbol, c_constructor_symbol, c_enum_variant, c_event_variant,
    c_event_variant_field, c_free_symbol, c_invalid_handle_macro, c_list_field,
    c_list_release_symbol, c_method_symbol, c_native_object_symbol, c_remove_listener_symbol,
    c_type_name, c_user_data_param, constructor_suffix, is_binding_accessor,
    struct_has_owned_fields, swift_method_name, TypeOrigins, STRING_FREE_FN, STRING_LIST_FREE_FN,
    STRING_MAP_FREE_FN,
};
use codegen_shared::GeneratedFile;
use codegen_shared::ir::{
    Api, Class, Constructor, Enum, EventGroup, Header, Method, Param, Struct, TypeRef,
};

pub fn generate(
    api: &Api,
    header: &Header,
    origins: &TypeOrigins,
    swift_out: &Path,
    prefix: &str,
    subdir: Option<&Path>,
) -> GeneratedFile {
    let file_name = format!("{}.swift", header.stem.to_upper_camel_case());
    // Swift mirrors the source tree but in its own casing: `src/foundation/`
    // becomes `Sources/NativeAPI/Foundation/`, which matters on a
    // case-sensitive filesystem.
    let out_path = match subdir {
        Some(dir) => {
            let mut path = swift_out.to_path_buf();
            for part in dir.components() {
                path.push(part.as_os_str().to_string_lossy().to_upper_camel_case());
            }
            path.join(&file_name)
        }
        None => swift_out.join(&file_name),
    };
    GeneratedFile {
        path: out_path,
        contents: generate_swift(api, header, origins, prefix),
    }
}

/// Swift has one namespace per module, so anything shared between generated
/// files has to be declared exactly once.
pub fn generate_support(api: &Api, swift_out: &Path) -> GeneratedFile {
    let mut out = String::new();
    writeln!(out, "// AUTO-GENERATED. DO NOT EDIT.").unwrap();
    writeln!(
        out,
        "// Any manual changes WILL BE LOST when this file is regenerated."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "import CNativeAPI").unwrap();
    writeln!(out, "import Foundation").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// Identifies one registered event listener.").unwrap();
    writeln!(out, "public typealias ListenerId = native_listener_id_t").unwrap();
    writeln!(out).unwrap();

    let mut arities: Vec<usize> = api
        .headers
        .iter()
        .flat_map(|header| callback_arities(header))
        .collect();
    // Every listener registration goes through a one-argument box.
    if api
        .headers
        .iter()
        .any(|header| header.classes.iter().any(|class| class.event.is_some()))
    {
        arities.push(1);
    }
    arities.sort_unstable();
    arities.dedup();
    for arity in arities {
        generate_swift_callback_box(&mut out, arity);
    }

    GeneratedFile {
        path: swift_out.join("Support.swift"),
        contents: out,
    }
}

fn generate_swift(api: &Api, header: &Header, origins: &TypeOrigins, prefix: &str) -> String {
    let mut out = String::new();
    writeln!(out, "// AUTO-GENERATED. DO NOT EDIT.").unwrap();
    writeln!(
        out,
        "// Any manual changes WILL BE LOST when this file is regenerated."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "import CNativeAPI").unwrap();
    writeln!(out, "import Foundation").unwrap();
    writeln!(out).unwrap();

    for alias in &header.aliases {
        if origins.get(&alias.name).map(String::as_str) != Some(header.stem.as_str()) {
            continue;
        }
        writeln!(
            out,
            "public typealias {} = {}",
            alias.name,
            swift_public_type(&alias.underlying)
        )
        .unwrap();
        writeln!(out).unwrap();
    }

    for item in &header.enums {
        generate_swift_enum(&mut out, item, prefix);
    }

    for item in &header.structs {
        generate_swift_struct(&mut out, item, prefix);
    }

    for group in &header.events {
        generate_swift_event(&mut out, group, prefix);
    }

    for class in &header.classes {
        if class.is_instance() {
            generate_swift_handle_class(&mut out, api, header, class, prefix);
        } else {
            // Singletons surface as `.shared` instances rather than static
            // namespaces so they can adopt protocols and be swapped in tests.
            // Sendable is sound: the wrapper is stateless, every method body
            // just forwards to the C ABI.
            writeln!(out, "public final class {}: Sendable {{", class.name).unwrap();
            writeln!(out, "  /// The shared instance backed by the native singleton.").unwrap();
            writeln!(out, "  public static let shared = {}()", class.name).unwrap();
            writeln!(out).unwrap();
            writeln!(out, "  private init() {{}}").unwrap();
            writeln!(out).unwrap();
            for method in &class.methods {
                generate_swift_method(&mut out, api, class, method, header, prefix, "  ");
            }
            generate_swift_listener(&mut out, api, class, prefix, "  ");
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Enums, structs, events
// ---------------------------------------------------------------------------

fn generate_swift_enum(out: &mut String, item: &Enum, prefix: &str) {
    let c_ty = c_type_name(prefix, &item.name);
    writeln!(out, "public enum {}: CInt {{", item.name).unwrap();
    for variant in &item.variants {
        writeln!(
            out,
            "  case {} = {}",
            swift_enum_case(&variant.name),
            variant.value
        )
        .unwrap();
    }
    writeln!(out).unwrap();
    let fallback = item
        .variants
        .first()
        .map(|variant| swift_enum_case(&variant.name))
        .unwrap_or_else(|| "unknown".to_string());
    writeln!(out, "  init(raw: {c_ty}) {{").unwrap();
    writeln!(out, "    switch raw {{").unwrap();
    for variant in &item.variants {
        writeln!(
            out,
            "    case {}: self = .{}",
            c_enum_variant(prefix, item, &variant.name),
            swift_enum_case(&variant.name)
        )
        .unwrap();
    }
    writeln!(out, "    default: self = .{fallback}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  var raw: {c_ty} {{").unwrap();
    writeln!(out, "    {c_ty}(rawValue: UInt32(self.rawValue))").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_swift_struct(out: &mut String, item: &Struct, prefix: &str) {
    let c_ty = c_type_name(prefix, &item.name);
    writeln!(out, "public struct {} {{", item.name).unwrap();
    for field in &item.fields {
        writeln!(
            out,
            "  public var {}: {}",
            field.name.to_lower_camel_case(),
            swift_field_type(&field.ty)
        )
        .unwrap();
    }
    write!(out, "\n  public init(").unwrap();
    for (index, field) in item.fields.iter().enumerate() {
        if index > 0 {
            write!(out, ", ").unwrap();
        }
        write!(
            out,
            "{}: {}",
            field.name.to_lower_camel_case(),
            swift_field_type(&field.ty)
        )
        .unwrap();
        if is_callback(&field.ty) {
            write!(out, " = nil").unwrap();
        }
    }
    writeln!(out, ") {{").unwrap();
    for field in &item.fields {
        let name = field.name.to_lower_camel_case();
        writeln!(out, "    self.{name} = {name}").unwrap();
    }
    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "extension {} {{", item.name).unwrap();
    writeln!(out, "  init(raw: {c_ty}) {{").unwrap();
    write!(out, "    self.init(").unwrap();
    let mut first = true;
    for field in &item.fields {
        if !first {
            write!(out, ", ").unwrap();
        }
        first = false;
        let name = field.name.to_lower_camel_case();
        let raw = field.name.to_snake_case();
        let value = match &field.ty {
            TypeRef::String | TypeRef::CString => {
                format!("raw.{raw}.map {{ String(cString: $0) }}")
            }
            TypeRef::Enum { name: enum_name, .. } => format!("{enum_name}(raw: raw.{raw})"),
            TypeRef::Struct {
                name: struct_name, ..
            } => format!("{struct_name}(raw: raw.{raw})"),
            TypeRef::Callback { .. } => "nil".to_string(),
            _ => format!("raw.{raw}"),
        };
        write!(out, "{name}: {value}").unwrap();
    }
    writeln!(out, ")").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    // The C form, plus a matching release for whatever it allocated. Kept as a
    // pair rather than a `withRaw` closure so a call taking several structs
    // stays flat instead of nesting one closure per argument.
    writeln!(out, "  func toRaw() -> {c_ty} {{").unwrap();
    writeln!(out, "    var raw = {c_ty}()").unwrap();
    for field in &item.fields {
        let name = field.name.to_lower_camel_case();
        let raw = field.name.to_snake_case();
        match &field.ty {
            TypeRef::String | TypeRef::CString => {
                writeln!(
                    out,
                    "    raw.{raw} = self.{name}.map {{ strdup($0) }} ?? nil"
                )
                .unwrap();
            }
            TypeRef::Enum { .. } => writeln!(out, "    raw.{raw} = self.{name}.raw").unwrap(),
            TypeRef::Struct { .. } => {
                writeln!(out, "    raw.{raw} = self.{name}.toRaw()").unwrap()
            }
            TypeRef::Callback { params } => {
                let user_data = c_user_data_param(&field.name);
                writeln!(out, "    if let callback = self.{name} {{").unwrap();
                writeln!(
                    out,
                    "      raw.{raw} = {}",
                    swift_trampoline_literal(params, "      ")
                )
                .unwrap();
                writeln!(
                    out,
                    "      raw.{user_data} = Unmanaged.passRetained(CallbackBox{}(callback)).toOpaque()",
                    params.len()
                )
                .unwrap();
                writeln!(out, "    }}").unwrap();
            }
            _ => writeln!(out, "    raw.{raw} = self.{name}").unwrap(),
        }
    }
    writeln!(out, "    return raw").unwrap();
    writeln!(out, "  }}").unwrap();

    if struct_has_owned_fields(item) {
        writeln!(out).unwrap();
        writeln!(out, "  static func releaseRaw(_ raw: inout {c_ty}) {{").unwrap();
        for field in &item.fields {
            if matches!(field.ty, TypeRef::String | TypeRef::CString) {
                let raw = field.name.to_snake_case();
                writeln!(out, "    free(raw.{raw})").unwrap();
                writeln!(out, "    raw.{raw} = nil").unwrap();
            }
        }
        writeln!(out, "  }}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_swift_event(out: &mut String, group: &EventGroup, prefix: &str) {
    writeln!(out, "/// One `{}`, in its concrete form.", group.name).unwrap();
    writeln!(out, "public enum {} {{", group.name).unwrap();
    for variant in &group.variants {
        let fields: Vec<String> = group
            .common
            .iter()
            .chain(variant.fields.iter())
            .map(|field| {
                format!(
                    "{}: {}",
                    field.name.to_lower_camel_case(),
                    swift_event_field_type(&field.ty)
                )
            })
            .collect();
        let case = variant.discriminant.to_lower_camel_case();
        if fields.is_empty() {
            writeln!(out, "  case {case}").unwrap();
        } else {
            writeln!(out, "  case {case}({})", fields.join(", ")).unwrap();
        }
    }
    writeln!(out).unwrap();

    let raw_ty = c_type_name(prefix, &group.name);
    writeln!(out, "  init?(raw: {raw_ty}) {{").unwrap();
    writeln!(out, "    switch raw.type {{").unwrap();
    for variant in &group.variants {
        let payload = c_event_variant_field(&variant.discriminant);
        let mut values: Vec<String> = Vec::new();
        for field in &group.common {
            values.push(format!(
                "{}: {}",
                field.name.to_lower_camel_case(),
                swift_event_field_expr(&field.ty, &format!("raw.{}", field.name.to_snake_case()))
            ));
        }
        for field in &variant.fields {
            values.push(format!(
                "{}: {}",
                field.name.to_lower_camel_case(),
                swift_event_field_expr(
                    &field.ty,
                    &format!("raw.data.{payload}.{}", field.name.to_snake_case())
                )
            ));
        }
        let case = variant.discriminant.to_lower_camel_case();
        let constructed = if values.is_empty() {
            format!(".{case}")
        } else {
            format!(".{case}({})", values.join(", "))
        };
        writeln!(
            out,
            "    case {}: self = {constructed}",
            c_event_variant(prefix, &group.name, &variant.discriminant)
        )
        .unwrap();
    }
    writeln!(out, "    default: return nil").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Event payload handles are borrowed: the C side releases them when the
/// callback returns, so the wrapper is built with `ownsHandle: false`.
fn swift_event_field_expr(ty: &TypeRef, access: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::CString => format!("{access}.map {{ String(cString: $0) }}"),
        TypeRef::Enum { name, .. } => format!("{name}(raw: {access})"),
        TypeRef::Struct { name, .. } => format!("{name}(raw: {access})"),
        TypeRef::Object { name, .. } => {
            format!("{name}(nativeHandle: {access}, ownsHandle: false)")
        }
        _ => access.to_string(),
    }
}

fn swift_event_field_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Object { name, .. } => name.clone(),
        other => swift_public_type(other),
    }
}

/// Distinct callback arities used anywhere in this header.
fn callback_arities(header: &Header) -> Vec<usize> {
    let mut arities: Vec<usize> = Vec::new();
    let note = |ty: &TypeRef, arities: &mut Vec<usize>| {
        if let TypeRef::Callback { params } = ty.unwrap_optional() {
            if !arities.contains(&params.len()) {
                arities.push(params.len());
            }
        }
    };
    for item in &header.structs {
        for field in &item.fields {
            note(&field.ty, &mut arities);
        }
    }
    for class in &header.classes {
        for ctor in &class.constructors {
            for param in &ctor.params {
                note(&param.ty, &mut arities);
            }
        }
        for method in &class.methods {
            for param in &method.params {
                note(&param.ty, &mut arities);
            }
        }
    }
    arities.sort_unstable();
    arities
}

/// A `@convention(c)` function cannot capture, so the Swift closure travels
/// through the C `user_data` pointer inside one of these boxes.
fn generate_swift_callback_box(out: &mut String, arity: usize) {
    let generics: Vec<String> = (0..arity).map(|index| format!("A{index}")).collect();
    let generic_list = if generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", generics.join(", "))
    };
    writeln!(out, "final class CallbackBox{arity}{generic_list} {{").unwrap();
    writeln!(
        out,
        "  let body: ({}) -> Void",
        generics.join(", ")
    )
    .unwrap();
    writeln!(
        out,
        "  init(_ body: @escaping ({}) -> Void) {{ self.body = body }}",
        generics.join(", ")
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ---------------------------------------------------------------------------
// Classes
// ---------------------------------------------------------------------------

fn generate_swift_handle_class(
    out: &mut String,
    api: &Api,
    header: &Header,
    class: &Class,
    prefix: &str,
) {
    let handle_type = c_type_name(prefix, &class.name);
    writeln!(out, "/// Owned handle to a native `{}`.", class.name).unwrap();
    writeln!(out, "public final class {} {{", class.name).unwrap();
    writeln!(out, "  public let nativeHandle: {handle_type}").unwrap();
    writeln!(out, "  private let ownsHandle: Bool").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "  public init(nativeHandle: {handle_type}, ownsHandle: Bool = true) {{"
    )
    .unwrap();
    writeln!(out, "    self.nativeHandle = nativeHandle").unwrap();
    writeln!(out, "    self.ownsHandle = ownsHandle").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  deinit {{").unwrap();
    writeln!(out, "    if ownsHandle {{").unwrap();
    writeln!(
        out,
        "      {}(nativeHandle)",
        c_free_symbol(prefix, &class.name)
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    for ctor in &class.constructors {
        generate_swift_constructor(out, api, class, ctor, prefix);
    }

    for method in &class.methods {
        generate_swift_method(out, api, class, method, header, prefix, "  ");
    }

    if class.native_object {
        writeln!(
            out,
            "  /// Platform-specific native object behind this handle."
        )
        .unwrap();
        writeln!(out, "  public var nativeObject: UnsafeMutableRawPointer? {{").unwrap();
        writeln!(
            out,
            "    {}(nativeHandle)",
            c_native_object_symbol(prefix, &class.name)
        )
        .unwrap();
        writeln!(out, "  }}").unwrap();
        writeln!(out).unwrap();
    }

    generate_swift_listener(out, api, class, prefix, "  ");

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_swift_constructor(
    out: &mut String,
    api: &Api,
    class: &Class,
    ctor: &Constructor,
    prefix: &str,
) {
    let label = match constructor_suffix(class, ctor) {
        Some(suffix) => format!("create{}", suffix.to_upper_camel_case()),
        None => "create".to_string(),
    };
    writeln!(
        out,
        "  /// Creates a new `{}`; returns nil if the native side failed.",
        class.name
    )
    .unwrap();
    writeln!(
        out,
        "  public static func {label}({}) -> {}? {{",
        swift_params(&ctor.params),
        class.name
    )
    .unwrap();
    render_param_bindings(out, api, &ctor.params, prefix, "    ");
    writeln!(
        out,
        "    let handle = {}({})",
        c_constructor_symbol(prefix, class, ctor),
        call_args(&ctor.params, None)
    )
    .unwrap();
    writeln!(
        out,
        "    guard handle != {} else {{ return nil }}",
        c_invalid_handle_macro(prefix, &class.name)
    )
    .unwrap();
    writeln!(out, "    return {}(nativeHandle: handle)", class.name).unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
}

fn generate_swift_method(
    out: &mut String,
    api: &Api,
    class: &Class,
    method: &Method,
    header: &Header,
    prefix: &str,
    indent: &str,
) {
    let instance = class.is_instance() && !method.is_static;
    let name = swift_method_name(class, method);
    // A parameterless const getter reads better as a property, but only when it
    // needs no local bindings: a computed property cannot host a `defer`.
    let accessor = is_binding_accessor(class, method) && method.params.is_empty();
    let body_indent = format!("{indent}  ");

    if accessor {
        writeln!(
            out,
            "{indent}public var {name}: {} {{",
            swift_public_type(&method.return_type)
        )
        .unwrap();
    } else {
        // Singleton methods are instance methods on `.shared`; only a static
        // method of a handle class stays static.
        let keyword = if class.is_instance() && method.is_static {
            "static func"
        } else {
            "func"
        };
        writeln!(
            out,
            "{indent}public {keyword} {name}({}) -> {} {{",
            swift_params(&method.params),
            swift_public_type(&method.return_type)
        )
        .unwrap();
    }

    render_param_bindings(out, api, &method.params, prefix, &body_indent);

    let receiver = instance.then(|| "nativeHandle".to_string());
    let args = call_args(&method.params, receiver);
    let symbol = c_method_symbol(prefix, class, method);

    render_return(
        out,
        &method.return_type,
        &symbol,
        &args,
        header,
        prefix,
        &body_indent,
    );

    writeln!(out, "{indent}}}").unwrap();
    writeln!(out).unwrap();
}

fn generate_swift_listener(
    out: &mut String,
    api: &Api,
    class: &Class,
    prefix: &str,
    indent: &str,
) {
    let Some(group) = emitted_group(api, class) else {
        return;
    };
    let raw_ty = c_type_name(prefix, &group.name);
    let instance = class.is_instance();
    // Listener registration is an instance method everywhere: handle classes
    // pass their handle, singletons go through `.shared`.
    let keyword = "func";
    let self_arg = if instance { "nativeHandle, " } else { "" };

    writeln!(
        out,
        "{indent}/// Registers `callback` for every `{}` this `{}` emits.",
        group.name, class.name
    )
    .unwrap();
    writeln!(out, "{indent}///").unwrap();
    writeln!(
        out,
        "{indent}/// The closure is retained for good: the C ABI keeps the context"
    )
    .unwrap();
    writeln!(
        out,
        "{indent}/// pointer but offers no hook to release it, so removing the listener"
    )
    .unwrap();
    writeln!(out, "{indent}/// stops the calls without freeing the closure.").unwrap();
    writeln!(
        out,
        "{indent}public {keyword} addListener(_ callback: @escaping ({}) -> Void) -> ListenerId {{",
        group.name
    )
    .unwrap();
    writeln!(
        out,
        "{indent}  let context = Unmanaged.passRetained(CallbackBox1<{}>(callback)).toOpaque()",
        group.name
    )
    .unwrap();
    writeln!(
        out,
        "{indent}  let trampoline: @convention(c) (UnsafePointer<{raw_ty}>?, UnsafeMutableRawPointer?) -> Void = {{ event, userData in"
    )
    .unwrap();
    writeln!(
        out,
        "{indent}    guard let event, let userData, let value = {}(raw: event.pointee) else {{ return }}",
        group.name
    )
    .unwrap();
    writeln!(
        out,
        "{indent}    Unmanaged<CallbackBox1<{}>>.fromOpaque(userData).takeUnretainedValue().body(value)",
        group.name
    )
    .unwrap();
    writeln!(out, "{indent}  }}").unwrap();
    writeln!(
        out,
        "{indent}  return {}({self_arg}trampoline, context)",
        c_add_listener_symbol(prefix, &class.name)
    )
    .unwrap();
    writeln!(out, "{indent}}}").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "{indent}/// Unregisters a listener. Returns false if unknown."
    )
    .unwrap();
    writeln!(
        out,
        "{indent}public {keyword} removeListener(_ listenerId: ListenerId) -> Bool {{"
    )
    .unwrap();
    writeln!(
        out,
        "{indent}  {}({self_arg}listenerId)",
        c_remove_listener_symbol(prefix, &class.name)
    )
    .unwrap();
    writeln!(out, "{indent}}}").unwrap();
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
// Parameters and returns
// ---------------------------------------------------------------------------

fn swift_params(params: &[Param]) -> String {
    params
        .iter()
        .map(|param| {
            format!(
                "{}: {}",
                param.name.to_lower_camel_case(),
                swift_param_type(&param.ty)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn swift_param_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::CString => "String".to_string(),
        // A shared_ptr parameter accepts nil: that is how the C++ API clears an
        // icon or detaches a submenu.
        TypeRef::Object { name, shared: true, .. } => format!("{name}?"),
        TypeRef::Object { name, .. } => name.clone(),
        TypeRef::Vector { element } if matches!(element.as_ref(), TypeRef::String) => {
            "[String]".to_string()
        }
        TypeRef::Map { .. } => "[String: String]".to_string(),
        TypeRef::Callback { params } => format!("@escaping ({}) -> Void", swift_callback_args(params)),
        TypeRef::Optional { inner } => match inner.as_ref() {
            TypeRef::Callback { params } => {
                format!("(({}) -> Void)?", swift_callback_args(params))
            }
            other => format!("{}?", swift_param_type(other)),
        },
        other => swift_public_type(other),
    }
}

fn swift_callback_args(params: &[TypeRef]) -> String {
    params
        .iter()
        .map(swift_public_type)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `@convention(c)` shim for a callback of this shape.
fn swift_trampoline_literal(params: &[TypeRef], indent: &str) -> String {
    let arity = params.len();
    let c_args: Vec<String> = params
        .iter()
        .map(|param| swift_c_param_type(param))
        .chain(std::iter::once("UnsafeMutableRawPointer?".to_string()))
        .collect();
    let names: Vec<String> = (0..arity)
        .map(|index| format!("arg{index}"))
        .chain(std::iter::once("userData".to_string()))
        .collect();
    let generics = if arity == 0 {
        String::new()
    } else {
        format!(
            "<{}>",
            params
                .iter()
                .map(swift_public_type)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let call_args: Vec<String> = (0..arity).map(|index| format!("arg{index}")).collect();
    format!(
        "{{ ({}) -> Void in\n{indent}  guard let userData else {{ return }}\n{indent}  Unmanaged<CallbackBox{arity}{generics}>.fromOpaque(userData).takeUnretainedValue().body({})\n{indent}}} as @convention(c) ({}) -> Void",
        names
            .iter()
            .zip(c_args.iter())
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", "),
        call_args.join(", "),
        c_args.join(", ")
    )
}

/// How a callback argument arrives from C.
fn swift_c_param_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Bool => "Bool".to_string(),
        TypeRef::Int { name } => swift_int_type(name),
        TypeRef::Float { name } => swift_float_type(name),
        TypeRef::Alias { underlying, .. } => swift_c_param_type(underlying),
        other => swift_public_type(other),
    }
}

/// Locals the call needs: C strings, converted structs, retained closures.
fn render_param_bindings(
    out: &mut String,
    api: &Api,
    params: &[Param],
    prefix: &str,
    indent: &str,
) {
    for param in params {
        let name = param.name.to_lower_camel_case();
        match &param.ty {
            TypeRef::String | TypeRef::CString => {
                writeln!(out, "{indent}let {name}CString = strdup({name})").unwrap();
                writeln!(out, "{indent}defer {{ free({name}CString) }}").unwrap();
            }
            TypeRef::Struct { name: type_name, .. } => {
                writeln!(out, "{indent}var {name}Raw = {name}.toRaw()").unwrap();
                if struct_owns_memory(api, type_name) {
                    writeln!(
                        out,
                        "{indent}defer {{ {type_name}.releaseRaw(&{name}Raw) }}"
                    )
                    .unwrap();
                }
            }
            TypeRef::Vector { element } if matches!(element.as_ref(), TypeRef::String) => {
                writeln!(
                    out,
                    "{indent}var {name}Pointers = {name}.map {{ strdup($0) }}"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}defer {{ {name}Pointers.forEach {{ free($0) }} }}"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var {name}List = native_string_list_t(items: &{name}Pointers, count: Int(bitPattern: UInt({name}Pointers.count)))"
                )
                .unwrap();
            }
            TypeRef::Map { .. } => {
                writeln!(
                    out,
                    "{indent}var {name}Keys = {name}.keys.map {{ strdup($0) }}"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var {name}Values = {name}.values.map {{ strdup($0) }}"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}defer {{ {name}Keys.forEach {{ free($0) }}; {name}Values.forEach {{ free($0) }} }}"
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}var {name}Map = native_string_map_t(keys: &{name}Keys, values: &{name}Values, count: Int(bitPattern: UInt({name}Keys.count)))"
                )
                .unwrap();
            }
            TypeRef::Callback { params: args } => {
                writeln!(
                    out,
                    "{indent}let {name}Context = Unmanaged.passRetained(CallbackBox{}{}({name})).toOpaque()",
                    args.len(),
                    swift_box_generics(args)
                )
                .unwrap();
                writeln!(
                    out,
                    "{indent}let {name}Trampoline = {}",
                    swift_trampoline_literal(args, indent)
                )
                .unwrap();
            }
            TypeRef::Optional { inner } => match inner.as_ref() {
                TypeRef::String | TypeRef::CString => {
                    writeln!(
                        out,
                        "{indent}let {name}CString = {name}.map {{ strdup($0) }} ?? nil"
                    )
                    .unwrap();
                    writeln!(out, "{indent}defer {{ free({name}CString) }}").unwrap();
                }
                TypeRef::Struct {
                    name: type_name, ..
                } => {
                    // `&someOptional` would hand C a pointer to the Optional
                    // itself, so the payload gets its own allocation instead.
                    let c_ty = c_type_name(prefix, type_name);
                    writeln!(
                        out,
                        "{indent}let {name}Pointer = {name}.map {{ value -> UnsafeMutablePointer<{c_ty}> in"
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "{indent}  let pointer = UnsafeMutablePointer<{c_ty}>.allocate(capacity: 1)"
                    )
                    .unwrap();
                    writeln!(out, "{indent}  pointer.initialize(to: value.toRaw())").unwrap();
                    writeln!(out, "{indent}  return pointer").unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                    writeln!(out, "{indent}defer {{").unwrap();
                    writeln!(out, "{indent}  if let pointer = {name}Pointer {{").unwrap();
                    if struct_owns_memory(api, type_name) {
                        writeln!(
                            out,
                            "{indent}    {type_name}.releaseRaw(&pointer.pointee)"
                        )
                        .unwrap();
                    }
                    writeln!(out, "{indent}    pointer.deinitialize(count: 1)").unwrap();
                    writeln!(out, "{indent}    pointer.deallocate()").unwrap();
                    writeln!(out, "{indent}  }}").unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
                TypeRef::Callback { params: args } => {
                    writeln!(
                        out,
                        "{indent}let {name}Context = {name}.map {{ Unmanaged.passRetained(CallbackBox{}{}($0)).toOpaque() }}",
                        args.len(),
                        swift_box_generics(args)
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "{indent}let {name}Trampoline = {}",
                        swift_trampoline_literal(args, indent)
                    )
                    .unwrap();
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn swift_box_generics(params: &[TypeRef]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            params
                .iter()
                .map(swift_public_type)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Whether `toRaw` allocates for this struct, and so needs a matching release.
fn struct_owns_memory(api: &Api, name: &str) -> bool {
    api.headers
        .iter()
        .flat_map(|header| header.structs.iter())
        .any(|item| item.name == name && struct_has_owned_fields(item))
}

fn call_args(params: &[Param], receiver: Option<String>) -> String {
    let mut args: Vec<String> = receiver.into_iter().collect();
    for param in params {
        let name = param.name.to_lower_camel_case();
        match &param.ty {
            TypeRef::String | TypeRef::CString => args.push(format!("{name}CString")),
            TypeRef::Object { shared: true, .. } => {
                args.push(format!("{name}?.nativeHandle ?? 0"))
            }
            TypeRef::Object { .. } => args.push(format!("{name}.nativeHandle")),
            TypeRef::Struct { .. } => args.push(format!("{name}Raw")),
            TypeRef::Enum { .. } => args.push(format!("{name}.raw")),
            TypeRef::Vector { .. } => args.push(format!("{name}List")),
            TypeRef::Map { .. } => args.push(format!("{name}Map")),
            TypeRef::Callback { .. } => {
                args.push(format!("{name}Trampoline"));
                args.push(format!("{name}Context"));
            }
            TypeRef::Optional { inner } => match inner.as_ref() {
                TypeRef::String | TypeRef::CString => args.push(format!("{name}CString")),
                TypeRef::Object { .. } => args.push(format!("{name}?.nativeHandle ?? 0")),
                TypeRef::Struct { .. } => args.push(format!("{name}Pointer")),
                TypeRef::Callback { .. } => {
                    args.push(format!(
                        "{name}Context != nil ? {name}Trampoline : nil"
                    ));
                    args.push(format!("{name}Context"));
                }
                _ => args.push(name),
            },
            _ => args.push(name),
        }
    }
    args.join(", ")
}

fn render_return(
    out: &mut String,
    ty: &TypeRef,
    symbol: &str,
    args: &str,
    header: &Header,
    prefix: &str,
    indent: &str,
) {
    match ty {
        TypeRef::Void => {
            writeln!(out, "{indent}{symbol}({args})").unwrap();
        }
        TypeRef::String | TypeRef::CString => {
            writeln!(
                out,
                "{indent}guard let value = {symbol}({args}) else {{ return nil }}"
            )
            .unwrap();
            writeln!(out, "{indent}defer {{ {STRING_FREE_FN}(value) }}").unwrap();
            writeln!(out, "{indent}return String(cString: value)").unwrap();
        }
        TypeRef::Struct { name, .. } => {
            let owns_memory = header
                .structs
                .iter()
                .any(|item| item.name == *name && struct_has_owned_fields(item));
            if owns_memory {
                writeln!(out, "{indent}var raw = {symbol}({args})").unwrap();
                writeln!(out, "{indent}let result = {name}(raw: raw)").unwrap();
                writeln!(out, "{indent}{}(&raw)", c_free_symbol(prefix, name)).unwrap();
                writeln!(out, "{indent}return result").unwrap();
            } else {
                writeln!(out, "{indent}return {name}(raw: {symbol}({args}))").unwrap();
            }
        }
        TypeRef::Enum { name, .. } => {
            writeln!(out, "{indent}return {name}(raw: {symbol}({args}))").unwrap();
        }
        TypeRef::Object { name, .. } => {
            writeln!(out, "{indent}let handle = {symbol}({args})").unwrap();
            writeln!(
                out,
                "{indent}guard handle != {} else {{ return nil }}",
                c_invalid_handle_macro(prefix, name)
            )
            .unwrap();
            writeln!(out, "{indent}return {name}(nativeHandle: handle)").unwrap();
        }
        TypeRef::Vector { element } if matches!(element.as_ref(), TypeRef::String) => {
            writeln!(out, "{indent}var list = {symbol}({args})").unwrap();
            writeln!(out, "{indent}var items: [String] = []").unwrap();
            writeln!(out, "{indent}if let buffer = list.items {{").unwrap();
            writeln!(out, "{indent}  for index in 0..<Int(list.count) {{").unwrap();
            writeln!(
                out,
                "{indent}    guard let value = buffer[index] else {{ continue }}"
            )
            .unwrap();
            writeln!(out, "{indent}    items.append(String(cString: value))").unwrap();
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}}}").unwrap();
            writeln!(out, "{indent}{STRING_LIST_FREE_FN}(&list)").unwrap();
            writeln!(out, "{indent}return items").unwrap();
        }
        TypeRef::Vector { element } => {
            generate_swift_list_return(out, element, prefix, symbol, args, indent);
        }
        TypeRef::Map { .. } => {
            writeln!(out, "{indent}var raw = {symbol}({args})").unwrap();
            writeln!(out, "{indent}var entries: [String: String] = [:]").unwrap();
            writeln!(
                out,
                "{indent}if let keys = raw.keys, let values = raw.values {{"
            )
            .unwrap();
            writeln!(out, "{indent}  for index in 0..<Int(raw.count) {{").unwrap();
            writeln!(out, "{indent}    guard let key = keys[index] else {{ continue }}").unwrap();
            writeln!(
                out,
                "{indent}    let value = values[index].map {{ String(cString: $0) }} ?? \"\""
            )
            .unwrap();
            writeln!(out, "{indent}    entries[String(cString: key)] = value").unwrap();
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}}}").unwrap();
            writeln!(out, "{indent}{STRING_MAP_FREE_FN}(&raw)").unwrap();
            writeln!(out, "{indent}return entries").unwrap();
        }
        TypeRef::Optional { inner } => {
            render_return(out, inner, symbol, args, header, prefix, indent)
        }
        _ => {
            writeln!(out, "{indent}return {symbol}({args})").unwrap();
        }
    }
}

fn generate_swift_list_return(
    out: &mut String,
    element: &TypeRef,
    prefix: &str,
    symbol: &str,
    args: &str,
    indent: &str,
) {
    let name = match element {
        TypeRef::Object { name, .. } => name,
        _ => return,
    };
    let field = c_list_field(name);
    writeln!(out, "{indent}var list = {symbol}({args})").unwrap();
    writeln!(out, "{indent}var items: [{name}] = []").unwrap();
    writeln!(out, "{indent}if let buffer = list.{field} {{").unwrap();
    writeln!(out, "{indent}  for index in 0..<Int(list.count) {{").unwrap();
    writeln!(
        out,
        "{indent}    items.append({name}(nativeHandle: buffer[index]))"
    )
    .unwrap();
    writeln!(out, "{indent}  }}").unwrap();
    writeln!(out, "{indent}}}").unwrap();
    writeln!(
        out,
        "{indent}// The handles now belong to `items`; free just the array."
    )
    .unwrap();
    writeln!(
        out,
        "{indent}{}(&list)",
        c_list_release_symbol(prefix, name)
    )
    .unwrap();
    writeln!(out, "{indent}return items").unwrap();
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

fn swift_field_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Callback { params } => {
            format!("(({}) -> Void)?", swift_callback_args(params))
        }
        other => swift_public_type(other),
    }
}

fn swift_public_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Void => "Void".to_string(),
        TypeRef::Bool => "Bool".to_string(),
        TypeRef::Int { name } => swift_int_type(name),
        TypeRef::Float { name } => swift_float_type(name),
        TypeRef::String | TypeRef::CString => "String?".to_string(),
        TypeRef::Alias { name, .. } => name.clone(),
        TypeRef::Enum { name, .. } | TypeRef::Struct { name, .. } => name.clone(),
        // Handles can always be null on the C side.
        TypeRef::Object { name, .. } => format!("{name}?"),
        TypeRef::Vector { element } => format!("[{}]", swift_element_type(element)),
        TypeRef::Map { .. } => "[String: String]".to_string(),
        TypeRef::Optional { inner } => match inner.as_ref() {
            TypeRef::String | TypeRef::CString => "String?".to_string(),
            other => {
                let base = swift_public_type(other);
                if base.ends_with('?') {
                    base
                } else {
                    format!("{base}?")
                }
            }
        },
        TypeRef::RawPointer => "UnsafeMutableRawPointer?".to_string(),
        TypeRef::Callback { .. } | TypeRef::Unsupported { .. } => "Void".to_string(),
    }
}

fn swift_element_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Object { name, .. } => name.clone(),
        TypeRef::String | TypeRef::CString => "String".to_string(),
        other => swift_public_type(other),
    }
}

fn swift_enum_case(name: &str) -> String {
    name.strip_prefix('k').unwrap_or(name).to_lower_camel_case()
}

fn swift_int_type(name: &str) -> String {
    match name {
        "char" | "signed char" => "Int8",
        "short" => "Int16",
        "int" => "CInt",
        "long" => "CLong",
        "long long" => "Int64",
        "unsigned char" => "UInt8",
        "unsigned short" => "UInt16",
        "unsigned int" => "UInt32",
        "unsigned long" => "CUnsignedLong",
        "unsigned long long" => "UInt64",
        _ => "CInt",
    }
    .to_string()
}

fn swift_float_type(name: &str) -> String {
    match name {
        "float" => "Float",
        _ => "Double",
    }
    .to_string()
}

fn is_callback(ty: &TypeRef) -> bool {
    codegen_shared::naming::is_callback(ty)
}
