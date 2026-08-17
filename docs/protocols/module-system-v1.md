# WOML Module System Contracts v1

Status: frozen for MS0 on 2026-08-10. MS1 implements document validation and
immutable local-module resolution only. Imported code is deliberately not
executable until MS3.

## 1. Source document

Every source has exactly this outer grammar:

```text
woml-document = <woml> imports? workflow </woml>
imports       = <imports> module+ </imports>
module        = <module name=alias from=relative-source />
```

- `<woml>` and `<imports>` have no attributes.
- `<imports>` precedes the one required `<workflow>`.
- A direct `<workflow>` root is rejected with `WOML_EXPECTED_DOCUMENT_ROOT`.
- `<module>` is empty and has exactly `name` and `from`.
- Competing spellings such as `<require>` and `<import>` are unknown elements.
- `.woml` sources cannot be modules.

## 2. Alias and source grammar

An alias matches `[a-z][A-Za-z0-9]*`, is at most 128 characters, and is unique.
The following names are reserved: `http`, `db`, `storage`, `cache`, `events`,
`queue`, `workflows`, `context`, `attempt`, `services`, and `secrets`.

A module source:

- begins with `./` or `../`;
- uses POSIX `/` separators;
- names a `.js` or `.ts` file explicitly;
- contains no URL query/fragment, NUL, absolute, home-relative, or remote form;
- resolves inside the configured project root both lexically and after symlink
  resolution; and
- cannot be declared twice under different aliases.

Static transitive relative imports follow the same extension and boundary
rules. Dynamic imports, package imports, cycles, case-only path collisions, and
missing files are rejected before activation. Package imports begin in MS5.

## 3. Export profile

The public v1 surface consists only of direct named function declarations:

```js
export function parse(value) {}
export async function write(value) {}
```

Type-only exports are ignored. Default exports, export lists, renamed or star
re-exports, exported values/classes/generators/proxies, and CommonJS exports are
rejected. Every declared entry point exports at least one accepted function.
Transitive files may contain internal runtime exports; only a declared module
entry point becomes `services.<alias>`.

## 4. Definition package manifest

The MS1 manifest conforms to
`docs/schemas/woml-definition-package.v1.schema.json`. It is explicitly marked
`executable: false`. It records:

- the workflow source and ID;
- sorted aliases, entry points, and public exports;
- every source's portable project-relative identity, exact-byte SHA-256 digest,
  media type, and sorted dependency paths;
- compiler/resolver profile identity;
- reviewed permission inputs (empty in MS1); and
- `rootHash`, computed as SHA-256 of canonical JSON for every manifest member
  except `rootHash` itself.

Canonical JSON sorts object keys recursively, preserves array order, and emits
ordinary JSON primitives without whitespace. Filesystem timestamps,
permissions, discovery order, absolute paths, cache locations, and inode data
are not hash inputs. Source byte changes, graph changes, aliases, exports,
compiler identity, or permission inputs change the root hash.

## 5. Diagnostics

Module failures use the existing WOML diagnostic envelope: `code`, `phase`,
`message`, `file`, source `location`, and optional `hint`. Declaration errors
point to the importing `.woml` attribute. Source/export/import errors point to
the portable module source. Important stable codes include:

- `WOML_EXPECTED_DOCUMENT_ROOT`
- `WOML_DOCUMENT_STRUCTURE_INVALID`
- `WOML_MODULE_ALIAS_INVALID`
- `WOML_MODULE_ALIAS_RESERVED`
- `WOML_MODULE_PATH_INVALID`
- `WOML_MODULE_PATH_ESCAPE`
- `WOML_MODULE_SYMLINK_ESCAPE`
- `WOML_MODULE_FILE_NOT_FOUND`
- `WOML_MODULE_GRAPH_CYCLE`
- `WOML_MODULE_DYNAMIC_IMPORT_UNSUPPORTED`
- `WOML_MODULE_PACKAGE_IMPORT_UNAVAILABLE`
- `WOML_MODULE_EXPORT_UNSUPPORTED`
- `WOML_MODULE_DEFAULT_EXPORT_UNSUPPORTED`
- `WOML_MODULE_COMMONJS_UNSUPPORTED`
- `WOML_MODULE_EXPORTS_EMPTY`
- `WOML_MODULE_EXECUTION_UNAVAILABLE`

## 6. Frozen limits and deferred contracts

MS1 limits a document to 64 declared aliases, the graph to 512 source files,
and total module source to 16 MiB. Enforcement of runtime initialization,
timeouts, source maps, immutable artifact storage, permissions, isolated module
state, and Script Host/Rust messages begins in later MS phases and must use new
versioned artifacts. This contract does not mutate Model v8, Script Host v4,
Capability Call v1, or Run Event v2.

No user-authored pure/capability kind exists. Modules receive arguments
explicitly and do not automatically receive `context`, `attempt`, or `secrets`.
`.woml` imports and workflow composition remain out of scope;
`services.workflows.call()` is the next product milestone.

## 7. CLI contract

MS1 publishes `woml check <workflow.woml> [--json]`:

- without `--json`, it prints the workflow ID, root hash, aliases, public
  functions, and dependency count;
- with `--json`, it prints the exact manifest; and
- it never activates triggers or executes module code.

The CLI chooses the nearest ancestor containing `woml.json` or `package.json`
as the project boundary, preferring `woml.json` when both exist. If neither is
present, the importing file's directory is the boundary. The programmatic API
accepts an explicit `projectRoot` for build systems and tests.

`woml run` continues to execute module-free documents. A document declaring a
module fails closed with `WOML_MODULE_EXECUTION_UNAVAILABLE` until MS3.
