"""Inspect selected types or Type::Method bodies in a supplied CLI assembly."""
import argparse
from pathlib import Path

import dnfile
from dncil.cil.body.reader import read_method_body_from_bytes
from dncil.clr.token import StringToken, Token


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assembly", type=Path)
    parser.add_argument("selectors", nargs="+")
    args = parser.parse_args()
    pe = dnfile.dnPE(str(args.assembly), clr_lazy_load=True)
    md = pe.net.mdtables
    assembly = md.Assembly.rows[0]
    print("Assembly", assembly.Name, ".".join(str(getattr(assembly, part))
          for part in ("MajorVersion", "MinorVersion", "BuildNumber", "RevisionNumber")))
    owners = {}
    types = {}
    for ty in md.TypeDef:
        name = f"{ty.TypeNamespace}.{ty.TypeName}".lstrip(".")
        types[name] = ty
        for table_name, indices in (("MethodDef", ty.MethodList), ("Field", ty.FieldList)):
            for idx in indices:
                owners[table_name, idx.row_index] = name

    def operand(value):
        if isinstance(value, StringToken):
            return repr(str(pe.net.user_strings.get(value.rid)))
        if isinstance(value, Token):
            table = md.tables[value.table]
            row = table.get_with_row_index(value.rid)
            if hasattr(row, "TypeName"):
                return f"{row.TypeNamespace}.{row.TypeName}"
            name = str(getattr(row, "Name", value))
            owner = owners.get((table.name, value.rid))
            if table.name == "MemberRef":
                parent = row.Class.row
                owner = str(getattr(parent, "TypeName", ""))
            sig = getattr(row, "Signature", None)
            if not owner:
                return name
            signature = f" [{sig.value.hex()}]" if sig else ""
            return f"{owner}::{name}{signature}"
        return "" if value is None else str(value)

    for selector in args.selectors:
        name, _, method = selector.partition("::")
        if name not in types:
            raise SystemExit(f"Unknown type {name}")
        ty = types[name]
        print(f"\n{name}")
        if not method:
            print("Fields:", " ".join(str(i.row.Name) for i in ty.FieldList))
            print("Methods:", " ".join(str(i.row.Name) for i in ty.MethodList))
            continue
        matches = [i.row for i in ty.MethodList if str(i.row.Name) == method]
        if not matches:
            raise SystemExit(f"Unknown method {selector}")
        for row in matches:
            print(f"{selector} RVA={row.Rva:#x}")
            body = read_method_body_from_bytes(pe.get_data(row.Rva))
            for ins in body.instructions:
                print(f"{ins.offset:5d} {ins.opcode.name:14s} {operand(ins.operand)}")


if __name__ == "__main__":
    main()
