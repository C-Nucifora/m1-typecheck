#!/usr/bin/env python3
"""Merge M1 Build help-pane captures into m1-intrinsics.json.

Usage: python3 assets/merge-help-captures.py <captures-dir>

<captures-dir> holds the reconstructed help-pane references
(LibraryFunctions.md, m1_help_tables.md, m1_help_classes.md). Existing
entries in m1-intrinsics.json win — they carry curated flags (stateful /
deprecated / calibrationOnly) and overload unions the captures lack; the
captures contribute missing libraries/functions, the builtin enumeration
catalogue ("enums") and class descriptions ("classes"). Idempotent.
"""
import json, re, sys, pathlib

TYPE_MAP = {
    "Floating Point": "FloatingPoint", "Integer": "Integer",
    "Unsigned Integer": "UnsignedInteger", "Boolean": "Boolean",
    "String": "String", "Void": "Void", "Fixed Point 7dps": "FixedPoint7dps",
}

def parse_functions(path):
    src = path.read_text()
    libs = {}
    # Split per-function sections: ### Lib.Fn ... up to next ###/##.
    for sec in re.split(r'\n(?=### )', src):
        m = re.match(r'### ([A-Za-z0-9]+)\.([A-Za-z0-9 ]+)\n', sec)
        if not m:
            continue
        lib, fn = m.group(1), m.group(2).strip()
        sig = re.search(r'```\n(.+?)\n```', sec)
        if not sig:
            continue
        sm = re.match(r'^([A-Za-z0-9 ]+?) [A-Za-z0-9]+\.[A-Za-z0-9 ]+\((.*)\)$', sig.group(1).strip())
        ret, argstr = sm.groups()
        params = []
        arg_docs = dict(re.findall(r'\| `([^`]+)` \| ([^|]*) \|', sec))
        for a in filter(None, (a.strip() for a in argstr.split(','))) if argstr.strip() else []:
            am = re.match(r'^(.+) ([A-Za-z0-9_ ]+)$', a)
            ty, name = am.group(1).strip(), am.group(2).strip()
            params.append({"name": name, "type": TYPE_MAP[ty], "doc": arg_docs.get(name, "").strip()})
        # Doc: the summary line right under the heading + the prose after Returns.
        lines = [l.strip() for l in sec.splitlines()]
        summary = next((l for l in lines[1:] if l and not l.startswith(('```', '|', '**', '#'))), "")
        libs.setdefault(lib, []).append({
            "name": fn, "returns": TYPE_MAP[ret.strip()], "params": params,
            "doc": summary, "stateful": False, "deprecated": False,
        })
    return libs

def parse_enums(path):
    src = path.read_text()
    enums = []
    enum_zone = src.split("## Enumerations", 1)[1]
    for sec in re.split(r'\n(?=### )', enum_zone):
        m = re.match(r'### (.+)\n', sec)
        if not m or '|' not in sec:
            continue
        name = m.group(1).strip()
        members = []
        for row in re.findall(r'\| *(-?\d+) *\| *([^|]+?) *\| *([^|]*?) *\| *([^|]*?) *\|', sec):
            val, mname, severity, doc = row
            if mname == "Name":
                continue
            members.append({"name": mname, "value": int(val),
                            **({"severity": severity} if severity else {}),
                            **({"doc": doc} if doc else {})})
        if members:
            enums.append({"name": name, "members": members})
    return enums

def parse_classes(path):
    src = path.read_text()
    classes = {}
    for sec in re.split(r'\n(?=## )', src):
        m = re.match(r'## (.+)\n', sec)
        if not m or m.group(1).startswith('#'):
            continue
        name = m.group(1).strip()
        paras = [p.strip() for p in sec.split('\n\n')[1:] if p.strip()]
        if paras:
            classes[name] = paras[0]
    return classes

def main():
    cap = pathlib.Path(sys.argv[1])
    jpath = pathlib.Path(__file__).parent / "m1-intrinsics.json"
    j = json.loads(jpath.read_text())

    libs = parse_functions(cap / "LibraryFunctions.md")
    added_fns = added_libs = 0
    for lib, fns in sorted(libs.items()):
        dest = j["library"].setdefault(lib, {"doc": "", "functions": []})
        if not dest["functions"]:
            added_libs += 1
        have = {f["name"] for f in dest["functions"]}
        for f in fns:
            if f["name"] in have:
                # Existing entry wins; only fill an empty doc from the capture.
                for e in dest["functions"]:
                    if e["name"] == f["name"] and not e.get("doc"):
                        e["doc"] = f["doc"]
                continue
            dest["functions"].append(f)
            added_fns += 1

    j["enums"] = parse_enums(cap / "m1_help_tables.md")
    j["classes"] = parse_classes(cap / "m1_help_classes.md")
    j.setdefault("source", {})["helpCaptures"] = "M1 Build help-pane captures (M1_LIBRARIES_ENUMS_TYPES, 2026-06-11)"

    jpath.write_text(json.dumps(j, indent=1, ensure_ascii=False) + "\n")
    print(f"libraries added: {added_libs}, functions added: {added_fns}, "
          f"enums: {len(j['enums'])}, classes: {len(j['classes'])}")

if __name__ == "__main__":
    main()
