export type Leaf = {
  path: string;
  kind: "string" | "parametric" | "enum" | "option" | "repeating";
  link?: string;
  enumName?: string;
  optionKey?: string;
  single?: boolean;
  relPropUnique?: string[];
  filter?: unknown[];
};

const VAR_PREFIX = "av_p_vars.";

function enumName(enm: string, vars: Record<string, unknown>): string {
  const key = enm.startsWith(VAR_PREFIX) ? enm.slice(VAR_PREFIX.length) : enm;
  const variable = vars[key];
  if (
    variable &&
    typeof variable === "object" &&
    "name" in variable &&
    typeof variable.name === "string"
  ) {
    return variable.name;
  }
  return key;
}

function optionKey(opt: string): string {
  return opt.startsWith(VAR_PREFIX) ? opt.slice(VAR_PREFIX.length) : opt;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function collectLeaves(struct: unknown, vars: Record<string, unknown>): Map<string, Leaf> {
  const leaves = new Map<string, Leaf>();
  if (!isRecord(struct)) return leaves;
  for (const [key, child] of Object.entries(struct)) {
    visit(child, key, vars, leaves);
  }
  return leaves;
}

function visit(
  node: unknown,
  path: string,
  vars: Record<string, unknown>,
  leaves: Map<string, Leaf>,
) {
  if (typeof node === "string") {
    leaves.set(path, { path, kind: "string" });
    return;
  }
  if (!isRecord(node)) return;

  if (node.jvenum_field === true) {
    leaves.set(path, {
      path,
      kind: "enum",
      enumName: enumName(String(node.enm ?? ""), vars),
    });
    return;
  }
  if (node.jvopt_field === true) {
    leaves.set(path, {
      path,
      kind: "option",
      optionKey: optionKey(String(node.opt ?? "")),
      single: node.sngl === true,
    });
    return;
  }
  if (node.parametric === true && isRecord(node.possible_links) && "str" in node.possible_links) {
    leaves.set(path, { path, kind: "parametric", link: "str" });
    return;
  }

  const links = isRecord(node.possible_links) ? node.possible_links : undefined;
  if (Array.isArray(node.rel_prop_unique) || (links && "_filter_fields_" in links)) {
    leaves.set(path, {
      path,
      kind: "repeating",
      relPropUnique: Array.isArray(node.rel_prop_unique)
        ? node.rel_prop_unique.map(String)
        : undefined,
      filter: links && Array.isArray(links._filter_fields_) ? links._filter_fields_ : undefined,
    });
  }
  if (!links) return;
  for (const [key, child] of Object.entries(links)) {
    if (key === "jvrel_prop" || key === "_filter_fields_") continue;
    visit(child, `${path}.${key}`, vars, leaves);
  }
}

export function leafAt(struct: unknown, vars: Record<string, unknown>, path: string): Leaf {
  const leaf = collectLeaves(struct, vars).get(path);
  if (!leaf) throw new Error(`missing path: ${path}`);
  return leaf;
}
