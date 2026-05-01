import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const SRC_ROOT = path.resolve(process.cwd(), "src");
const EXTENSIONS = [".ts", ".tsx", ".js", ".mjs", ".cjs"];

function resolveAliasPath(specifier) {
  const rel = specifier.slice(2);
  const base = path.resolve(SRC_ROOT, rel);

  if (fs.existsSync(base) && fs.statSync(base).isFile()) {
    return base;
  }

  for (const ext of EXTENSIONS) {
    const candidate = `${base}${ext}`;
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return candidate;
    }
  }

  for (const ext of EXTENSIONS) {
    const candidate = path.join(base, `index${ext}`);
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return candidate;
    }
  }

  return `${base}.ts`;
}

export async function resolve(specifier, context, defaultResolve) {
  if (specifier.startsWith("@/")) {
    const resolvedPath = resolveAliasPath(specifier);
    return defaultResolve(pathToFileURL(resolvedPath).href, context, defaultResolve);
  }
  if ((specifier.startsWith("./") || specifier.startsWith("../")) && context.parentURL?.startsWith("file:")) {
    const parentPath = new URL(context.parentURL).pathname;
    const base = path.resolve(path.dirname(parentPath), specifier);
    for (const ext of EXTENSIONS) {
      const candidate = `${base}${ext}`;
      if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
        return defaultResolve(pathToFileURL(candidate).href, context, defaultResolve);
      }
    }
  }
  return defaultResolve(specifier, context, defaultResolve);
}
