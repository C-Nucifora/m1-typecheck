//! DBC reference checks (T108/T109/T110) — a per-script pass that validates
//! CAN-DBC reference chains against the project's augmented DBC symbol model.
//!
//! The `.m1dbc` augment (`crate::symbols::m1dbc`) registers each CAN object:
//! a module (`BuiltIn.CAN.DBC`, at bare `<Module>`), a message
//! (`BuiltIn.CAN.Message`, at `<Module>.<Message>`) and a signal
//! (`BuiltIn.CAN.Signal`, at `<Module>.<Message>.<Signal>`, typed from
//! `Props/@Type`). Scripts reference them either bare
//! (`BMU.ExtendedPackStatus.Receive()`) or DBC-qualified
//! (`DBC.BMU.ExtendedPackStatus.…`, rooted at the `BuiltIn.CAN.DBCRoot` group
//! named `DBC`). Both spellings are handled here.
//!
//! Why a dedicated pass and not the generic resolver: a DBC-qualified chain
//! resolves as `Resolution::Opaque` today because the module object
//! (`DBC.BMU`) exists and absorbs *any* trailing segments as opaque accessors
//! — so a renamed-away module or a typo'd message/signal is silently accepted
//! (M1 Build rejects it: Errors 1338/1352). This pass resolves the chain
//! *against the DBC tree itself* and reports the miss, while staying silent on
//! non-DBC roots (the resolver's "opaque roots stay opaque" guard).
//!
//! Three checks, each an **Error** (M1 Build rejects the build), run over the
//! same walk:
//!
//! - **T108 dbc-unresolved-reference** — a DBC-qualified module that does not
//!   exist, or a message/signal segment that does not exist under a known
//!   module (M1 Build 1338/1352 "does not exist").
//! - **T109 dbc-message-direction** — a message method that contradicts the
//!   message's `Transmit` direction: `.Receive()` on a `TX` message, or
//!   `.Transmit()`/`.Tx*()` on an `RX` message.
//! - **T110 dbc-signal-accessor-type** — a raw signal accessor whose value
//!   category disagrees with the signal's declared `Type`: an integer accessor
//!   (`Get/SetInteger`, `Get/SetUnsignedInteger`) on a float signal, a float
//!   accessor (`Get/SetFloat`) on an integer/bool signal, or a bit accessor
//!   (`Get/SetBit`) on a non-bool signal. The *scaled/physical* accessors
//!   (`GetScaled`, `SetScaled`, `SetFromBaseUnit`, …) are valid on any signal
//!   and are never constrained.
//!
//! False-positive safety (this is a CI gate; both real corpora pass M1 Build):
//! - A **bare** chain is examined only when its root is a *known* DBC module —
//!   an unknown bare root is indistinguishable from a legitimately-opaque
//!   firmware/external root, so it stays silent (the resolver's guard).
//! - An unresolved segment is reported only in a **data position** — i.e. when
//!   at least one further segment follows it (`Module.BadMsg.Receive`), so a
//!   one-segment tail (which could be a built-in object/message accessor like
//!   `.Init`) is never mistaken for a typo.
//! - Direction (T109) and type (T110) fire only when *both* sides are known:
//!   a message with no `Transmit`, or a signal whose `Type` is un-modelled
//!   (`ValueType::Unknown`), is left alone (the `Unknown`-is-absorbing guard).

use crate::diagnostics::{TypeCode, TypeDiagnostic, make};
use crate::resolve::Scope;
use crate::symbols::{CanDirection, SymbolTable};
use crate::typer::path_text;
use crate::types::ValueType;
use m1_core::{Kind, Node, Severity};
use std::collections::{HashMap, HashSet};

/// The DBC symbol model extracted once per run from the project table.
struct DbcModel {
    /// Bare single-segment module leaf names (`BMU`, `M150`) — a `BuiltIn.CAN.DBC`
    /// is registered both bare and as `DBC.<Name>`, so keying by leaf unifies them.
    modules: HashSet<String>,
    /// Full bare message paths (`BMU.ExtendedPackStatus`) → transmit direction.
    messages: HashMap<String, Option<CanDirection>>,
    /// Full bare signal paths (`BMU.ExtendedPackStatus.BmuHardwareVersion`) →
    /// declared value type.
    signals: HashMap<String, ValueType>,
    /// Whether a `BuiltIn.CAN.DBCRoot` group named `DBC` exists — only then is a
    /// leading `DBC.` segment treated as the DBC namespace (guards against a
    /// project group that happens to be called `DBC`).
    has_dbc_root: bool,
}

impl DbcModel {
    fn build(table: &SymbolTable) -> DbcModel {
        let mut modules = HashSet::new();
        let mut messages = HashMap::new();
        let mut signals = HashMap::new();
        let mut has_dbc_root = false;
        for s in table.iter() {
            match s.classname.as_deref() {
                Some("BuiltIn.CAN.DBC") => {
                    modules.insert(leaf(&s.path).to_string());
                }
                Some("BuiltIn.CAN.Message") => {
                    messages.insert(s.path.clone(), s.can.as_ref().and_then(|c| c.transmit));
                }
                Some("BuiltIn.CAN.Signal") => {
                    signals.insert(s.path.clone(), s.value_type);
                }
                Some("BuiltIn.CAN.DBCRoot") if s.path == "DBC" => has_dbc_root = true,
                _ => {}
            }
        }
        DbcModel {
            modules,
            messages,
            signals,
            has_dbc_root,
        }
    }

    fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

/// The leaf (last `.`-segment) of a path — `DBC.BMU` and bare `BMU` share `BMU`.
fn leaf(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

/// A signal accessor's expected value category, or `None` for the unconstrained
/// scaled/physical accessors (`GetScaled`, `SetFromBaseUnit`, `SetState`, …),
/// which are valid on a signal of any type and so are never flagged.
///
/// Signed vs unsigned is deliberately *not* distinguished — the raw bits are the
/// same and M1 Build accepts `GetUnsignedInteger` on a signed raw signal — so
/// both fold into the single `Integer` category. Only the float/integer/bool
/// distinction (which M1 Build does reject) is checked.
fn accessor_category(accessor: &str) -> Option<ValueType> {
    match accessor {
        "GetFloat" | "SetFloat" => Some(ValueType::Float),
        "GetInteger" | "SetInteger" | "GetUnsignedInteger" | "SetUnsignedInteger" => {
            Some(ValueType::Integer)
        }
        "GetBit" | "SetBit" => Some(ValueType::Boolean),
        _ => None,
    }
}

/// Coarse value category of a signal for the accessor-type check: integer
/// (`Unsigned`/`Integer` fold together), float or bool. `None` for a signal
/// whose type is un-modelled — the check then stays silent.
fn signal_category(vt: ValueType) -> Option<ValueType> {
    match vt {
        ValueType::Float => Some(ValueType::Float),
        ValueType::Unsigned | ValueType::Integer => Some(ValueType::Integer),
        ValueType::Boolean => Some(ValueType::Boolean),
        _ => None,
    }
}

fn category_name(vt: ValueType) -> &'static str {
    match vt {
        ValueType::Float => "float",
        ValueType::Integer => "integer",
        ValueType::Boolean => "bit/boolean",
        _ => "value",
    }
}

/// Entry point: append T108/T109/T110 diagnostics for the DBC reference chains
/// in `root`. No-op without a project, and without any DBC objects in it.
pub fn check(root: Node, scope: &Scope, out: &mut Vec<TypeDiagnostic>) {
    let Some(project) = scope.project else {
        return;
    };
    let model = DbcModel::build(project.symbols());
    if model.is_empty() {
        return;
    }
    walk(root, &model, out);
}

fn walk(n: Node, model: &DbcModel, out: &mut Vec<TypeDiagnostic>) {
    // A call `<path>(…)`: check the callee chain, then descend only into the
    // arguments (a generated accessor can appear there too). The callee is fully
    // handled here, so it is deliberately not descended into.
    if n.kind() == Kind::CallExpression {
        if let Some(callee) = n
            .named_children()
            .into_iter()
            .find(|c| matches!(c.kind(), Kind::Identifier | Kind::MemberExpression))
        {
            check_chain(&callee, model, out);
        }
        if let Some(args) = n
            .children()
            .into_iter()
            .find(|c| c.kind() == Kind::ArgumentList)
        {
            walk(args, model, out);
        }
        return;
    }
    // A standalone member access `<Module>.<Message>.<Signal>` (a read). A member
    // chain resolves as a whole; do not descend into its prefixes.
    if n.kind() == Kind::MemberExpression {
        check_chain(&n, model, out);
        return;
    }
    for c in n.children() {
        walk(c, model, out);
    }
}

/// Validate one reference chain `node` (a member expression or identifier) against
/// the DBC model, emitting at most one diagnostic.
fn check_chain(node: &Node, model: &DbcModel, out: &mut Vec<TypeDiagnostic>) {
    let text = path_text(*node);
    // An `expand … to` template placeholder (`$(N)`) means this chain is the
    // pre-expansion source text — `CMU.Segment $(N).Receive()` names no real
    // message until M1 Build substitutes the counter. The expanded names cannot
    // be resolved here (the same template-awareness as the T031 fix, #246), so
    // template chains stay silent rather than reporting a false "does not
    // exist" (seen on the real AV-M1 corpus).
    if text.contains("$(") {
        return;
    }
    // Segment on `.`, trimming any whitespace a multi-line chain introduced, so
    // segments match the (clean) symbol-table keys.
    let mut segs: Vec<&str> = text.split('.').map(str::trim).collect();

    // DBC-qualified form: strip the `DBC` namespace root (only when a real
    // DBCRoot named `DBC` exists), then require the module to exist.
    let qualified = segs.first() == Some(&"DBC") && model.has_dbc_root;
    if qualified {
        segs.remove(0);
        let Some(module) = segs.first() else {
            return; // bare `DBC` (the DBCRoot) — nothing to resolve.
        };
        if !model.modules.contains(*module) {
            out.push(make(
                TypeCode::T108,
                node,
                Severity::Error,
                format!(
                    "unresolved CAN reference: DBC module `{module}` does not exist \
                     (M1 Build Error 1338/1352 \"does not exist\")"
                ),
            ));
            return;
        }
    } else {
        // Bare form: examine only chains rooted at a *known* DBC module. An
        // unknown bare root is indistinguishable from an opaque external root and
        // stays silent (the resolver's "opaque roots stay opaque" guard).
        match segs.first() {
            Some(m) if model.modules.contains(*m) => {}
            _ => return,
        }
    }

    // `segs` is now `[<module>, seg1, seg2, …]` with a known module at [0].
    // Longest known data prefix: module (1) < message (2) < signal (3).
    let mut known = 1usize;
    if segs.len() >= 2 && model.messages.contains_key(&join(&segs[..2])) {
        known = 2;
        if segs.len() >= 3 && model.signals.contains_key(&join(&segs[..3])) {
            known = 3;
        }
    }
    let rest = &segs[known..];

    // A segment sitting in a *data position* (a message slot when only the module
    // is known, a signal slot when the message is known) that is not a known
    // symbol — recognised by at least one further segment following it — is an
    // unresolved reference. A one-segment tail is an accessor call and is left to
    // the direction/type checks below.
    if rest.len() >= 2 && known < 3 {
        let bad = join(&segs[..known + 1]);
        let what = if known == 1 { "message" } else { "signal" };
        out.push(make(
            TypeCode::T108,
            node,
            Severity::Error,
            format!(
                "unresolved CAN reference: {what} `{bad}` does not exist \
                 (M1 Build Error 1338/1352 \"does not exist\")"
            ),
        ));
        return;
    }

    // A single trailing accessor on a known message (direction, T109) or a known
    // signal (accessor-vs-type, T110).
    if rest.len() == 1 {
        let accessor = rest[0];
        if known == 2 {
            check_direction(node, &join(&segs[..2]), accessor, model, out);
        } else if known == 3 {
            check_accessor_type(node, &join(&segs[..3]), accessor, model, out);
        }
    }
}

/// T109: a message method that contradicts the message's `Transmit` direction.
/// Silent when the direction is unknown (no `Transmit` on the `.m1dbc`).
fn check_direction(
    node: &Node,
    message_path: &str,
    accessor: &str,
    model: &DbcModel,
    out: &mut Vec<TypeDiagnostic>,
) {
    let Some(Some(dir)) = model.messages.get(message_path).copied() else {
        return;
    };
    // `.Receive()` needs Rx; `.Transmit()`/`.Tx*()` need Tx.
    let is_rx_method = accessor == "Receive";
    let is_tx_method = accessor == "Transmit" || accessor.starts_with("Tx");
    let bad = match dir {
        CanDirection::Tx if is_rx_method => Some((
            "receive method `.Receive()`",
            "a transmit (TX) message — use a `.Tx…()` method",
        )),
        CanDirection::Rx if is_tx_method => Some((
            "transmit method `.Tx…()`",
            "a receive (RX) message — use `.Receive()`",
        )),
        _ => None,
    };
    if let Some((method, expected)) = bad {
        out.push(make(
            TypeCode::T109,
            node,
            Severity::Error,
            format!(
                "CAN message `{message_path}` is {expected}: `.{accessor}()` is a {method} \
                 that does not match the message's Transmit direction"
            ),
        ));
    }
}

/// T110: a raw signal accessor whose value category disagrees with the signal's
/// declared type. Silent for the unconstrained scaled/physical accessors and for
/// a signal of un-modelled type.
fn check_accessor_type(
    node: &Node,
    signal_path: &str,
    accessor: &str,
    model: &DbcModel,
    out: &mut Vec<TypeDiagnostic>,
) {
    let Some(expected) = accessor_category(accessor) else {
        return;
    };
    let Some(actual) = model
        .signals
        .get(signal_path)
        .copied()
        .and_then(signal_category)
    else {
        return;
    };
    if expected != actual {
        out.push(make(
            TypeCode::T110,
            node,
            Severity::Error,
            format!(
                "CAN signal `{signal_path}` has a {} type, but `.{accessor}()` accesses it as \
                 {}; use the accessor matching the signal's declared Type",
                category_name(actual),
                category_name(expected),
            ),
        ));
    }
}

fn join(segs: &[&str]) -> String {
    segs.join(".")
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::TypeCode;
    use crate::project::Project;
    use crate::rules::check_script;
    use std::path::Path;

    /// Build a project from an in-memory `.m1prj` (DBCRoot + DBC objects) plus an
    /// in-memory `.m1dbc` (messages + signals) written to a temp file, then check
    /// `src` as one script and return the T-codes it produces.
    fn codes_for(prj_xml: &str, dbc_xml: &str, src: &str) -> Vec<String> {
        let dir = std::env::temp_dir();
        let uniq = format!("{}_{:p}", std::process::id(), src);
        let prj_path = dir.join(format!("m1tc_dbcref_{uniq}.m1prj"));
        let dbc_path = dir.join(format!("m1tc_dbcref_{uniq}.m1dbc"));
        std::fs::write(&prj_path, prj_xml).unwrap();
        std::fs::write(&dbc_path, dbc_xml).unwrap();
        let project = Project::load(&prj_path)
            .unwrap()
            .with_dbc(&dbc_path, "dbc/Test.m1dbc")
            .unwrap();
        let result = check_script(&project, Path::new("Demo.Update.m1scr"), src);
        let _ = std::fs::remove_file(&prj_path);
        let _ = std::fs::remove_file(&dbc_path);
        result
            .diagnostics
            .iter()
            .map(|d| d.code.as_str().to_string())
            .collect()
    }

    const PRJ: &str = r#"<?xml version="1.0"?>
<Project>
  <Component Classname="BuiltIn.GroupCompound" Name="Root"/>
  <Component Classname="BuiltIn.CAN.DBCRoot" Name="DBC"/>
  <Component Classname="BuiltIn.CAN.DBC" Name="DBC.BMU"/>
</Project>"#;

    // A DBC with one RX message + one TX message, an unsigned signal and a float
    // signal, mirroring the real corpus shapes.
    const DBC: &str = r#"<?xml version="1.0"?>
<DBC>
 <ComponentStream>
  <List>
   <Component Classname="BuiltIn.CAN.DBC" Name="BMU"/>
   <Component Classname="BuiltIn.CAN.Message" Name="BMU.Status"><Props CANId="600" DLC="8" Transmit="RX"/></Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="BMU.Status.Voltage"><Props Type="u32" StartBit="0" Length="16"/></Component>
   <Component Classname="BuiltIn.CAN.Signal" Name="BMU.Status.Temperature"><Props Type="f32" StartBit="16" Length="16"/></Component>
   <Component Classname="BuiltIn.CAN.Message" Name="BMU.Command"><Props CANId="601" DLC="8" Transmit="TX"/></Component>
  </List>
 </ComponentStream>
</DBC>"#;

    fn has(codes: &[String], c: TypeCode) -> bool {
        codes.iter().any(|s| s == c.as_str())
    }

    // ── T108 dbc-unresolved-reference ──────────────────────────────────────

    #[test]
    fn t108_flags_unknown_dbc_qualified_module() {
        // `DBC.Nope` — not a registered module.
        let codes = codes_for(PRJ, DBC, "if (DBC.Nope.Status.Receive()) { }\n");
        assert!(has(&codes, TypeCode::T108), "{codes:?}");
    }

    #[test]
    fn t108_flags_unknown_message_under_known_module() {
        // Known module BMU, unknown message `BadMessage`, with a following accessor.
        let codes = codes_for(PRJ, DBC, "if (DBC.BMU.BadMessage.Receive()) { }\n");
        assert!(has(&codes, TypeCode::T108), "{codes:?}");
    }

    #[test]
    fn t108_flags_unknown_signal_under_known_message() {
        let codes = codes_for(
            PRJ,
            DBC,
            "local v = DBC.BMU.Status.BadSignal.GetUnsignedInteger();\n",
        );
        assert!(has(&codes, TypeCode::T108), "{codes:?}");
    }

    #[test]
    fn t108_flags_bare_unknown_message_under_known_module() {
        // The AV-M1 renamed-away shape, bare: known module, dangling message.
        let codes = codes_for(PRJ, DBC, "if (BMU.Gone.Receive()) { }\n");
        assert!(has(&codes, TypeCode::T108), "{codes:?}");
    }

    #[test]
    fn t108_silent_on_valid_references() {
        // Every segment exists — no miss (and the RX message + unsigned signal
        // accessors are direction/type-correct, so T109/T110 stay quiet too).
        let codes = codes_for(
            PRJ,
            DBC,
            "if (DBC.BMU.Status.Receive()) {\n  local v = DBC.BMU.Status.Voltage.GetUnsignedInteger();\n}\n",
        );
        assert!(codes.is_empty(), "expected clean, got {codes:?}");
    }

    #[test]
    fn t108_silent_on_expand_template_chains() {
        // An `expand … to` template references pre-expansion names —
        // `CMU.Segment $(N)` (real AV-M1 shape) resolves to nothing until
        // M1 Build substitutes the counter, so template chains must stay
        // silent (the T031 template-awareness, #246). Both spellings.
        let codes = codes_for(
            PRJ,
            DBC,
            "expand (N) in (1, 2) to\n{\n  if (DBC.BMU.Segment $(N).Receive())\n  {\n    local v = BMU.Segment $(N).Voltage.GetUnsignedInteger();\n  }\n}\n",
        );
        assert!(
            codes.is_empty(),
            "template chains must not resolve: {codes:?}"
        );
    }

    #[test]
    fn t108_silent_on_unknown_bare_non_dbc_root() {
        // A bare chain whose root is not a known DBC module is an opaque external
        // reference — never flagged (would be a false positive on firmware refs).
        let codes = codes_for(PRJ, DBC, "if (SomeFirmware.Thing.Do()) { }\n");
        assert!(!has(&codes, TypeCode::T108), "{codes:?}");
    }

    // ── T109 dbc-message-direction ─────────────────────────────────────────

    #[test]
    fn t109_flags_receive_on_tx_message() {
        // BMU.Command is TX; `.Receive()` is a receive method → mismatch.
        let codes = codes_for(PRJ, DBC, "if (DBC.BMU.Command.Receive()) { }\n");
        assert!(has(&codes, TypeCode::T109), "{codes:?}");
    }

    #[test]
    fn t109_flags_transmit_on_rx_message() {
        // BMU.Status is RX; a `.Tx…()` method → mismatch.
        let codes = codes_for(PRJ, DBC, "local h = DBC.BMU.Status.TxOpen();\n");
        assert!(has(&codes, TypeCode::T109), "{codes:?}");
    }

    #[test]
    fn t109_silent_on_matching_direction() {
        // RX + `.Receive()` and TX + `.TxOpen()` are both correct.
        let ok_rx = codes_for(PRJ, DBC, "if (DBC.BMU.Status.Receive()) { }\n");
        assert!(!has(&ok_rx, TypeCode::T109), "{ok_rx:?}");
        let ok_tx = codes_for(PRJ, DBC, "local h = DBC.BMU.Command.TxOpen();\n");
        assert!(!has(&ok_tx, TypeCode::T109), "{ok_tx:?}");
    }

    // ── T110 dbc-signal-accessor-type ──────────────────────────────────────

    #[test]
    fn t110_flags_integer_accessor_on_float_signal() {
        // BMU.Status.Temperature is f32; `.GetUnsignedInteger()` is an integer
        // accessor → mismatch.
        let codes = codes_for(
            PRJ,
            DBC,
            "local t = DBC.BMU.Status.Temperature.GetUnsignedInteger();\n",
        );
        assert!(has(&codes, TypeCode::T110), "{codes:?}");
    }

    #[test]
    fn t110_flags_float_accessor_on_integer_signal() {
        // BMU.Status.Voltage is u32; `.SetFloat(…)` is a float accessor → mismatch.
        let codes = codes_for(PRJ, DBC, "DBC.BMU.Status.Voltage.SetFloat(1.0);\n");
        assert!(has(&codes, TypeCode::T110), "{codes:?}");
    }

    #[test]
    fn t110_silent_on_matching_accessor() {
        // Integer accessor on the unsigned signal is fine.
        let ok = codes_for(
            PRJ,
            DBC,
            "local v = DBC.BMU.Status.Voltage.GetUnsignedInteger();\n",
        );
        assert!(!has(&ok, TypeCode::T110), "{ok:?}");
    }

    #[test]
    fn t110_silent_on_scaled_accessor() {
        // `.GetScaled()` applies the multiplier/offset and is valid on any signal
        // regardless of raw type — never constrained.
        let codes = codes_for(
            PRJ,
            DBC,
            "local t = DBC.BMU.Status.Temperature.GetScaled();\n",
        );
        assert!(!has(&codes, TypeCode::T110), "{codes:?}");
    }
}
