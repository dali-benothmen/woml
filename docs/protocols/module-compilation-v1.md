# WOML Module Compilation v1

Status: frozen and implemented for Module System MS2 on 2026-08-10.

This contract turns a reviewed Definition Package v1 source graph into an
immutable Definition Package v2. It compiles executable artifacts but does not
register or run them. Runtime loading begins in MS3.

## 1. Compiler profile

Each declared module entry point is bundled independently with the Bun version
recorded in the package:

- target: `bun`;
- format: ESM;
- code splitting: disabled;
- source maps: external with `sourcesContent` retained;
- minification: disabled;
- environment inlining: disabled;
- unresolved runtime imports: disabled;
- package imports: unavailable until MS5; and
- output names: `modules/<alias>.mjs` and `modules/<alias>.mjs.map`.

The MS1 resolver remains authoritative for the input graph. Compilation repeats
the source-graph hash after bundling and fails if any source changed during the
build.

## 2. Canonical artifacts

Definition Package v2 contains exact UTF-8 content and SHA-256 identities for:

1. `workflow.compiled.v9.json` — canonical JSON Model v9;
2. one ESM bundle and source map for each sorted module alias; and
3. `types/services.generated.d.ts` — the workflow's imported service surface.

Bundle source comments and source-map `sources` use project-relative POSIX
identities. Bun's working-directory-sensitive debug IDs are removed. Absolute paths, temporary build directories, timestamps, file
permissions, discovery order, and filesystem metadata are forbidden hash
inputs. Source maps are canonical JSON and every retained source body must
match its MS1 digest.

Definition Package v2 uses `executable: true` to mean that executable ESM bytes
exist. `runtimeReady: false` permanently identifies this as the compilation-only
profile. MS3 promotes the unchanged artifacts into Definition Package v3 before
activation; Package v2 itself is never reinterpreted or widened.

## 3. Model v9

Compiled Model v9 adds one top-level `moduleRuntime` object:

```json
{
  "profileVersion": 1,
  "modules": [
    {
      "name": "spreadsheet",
      "bundleDigest": "sha256:...",
      "sourceMapDigest": "sha256:...",
      "exports": ["read", "removeEmptyRows"]
    }
  ]
}
```

The model contains artifact identities and public export names only. It never
contains module source, bundles, maps, absolute paths, secret values, or Bun
loader instructions. Existing graph, trigger, retry, approval, parallel,
branch, and Script Bindings v1 contracts remain unchanged.

## 4. Generated declarations

MS2 emits a read-only `WomlImportedServices` interface and a matching `services`
binding. Each accepted public function is represented conservatively as:

```ts
(...args: readonly unknown[]) => unknown | Promise<unknown>
```

Precise signature inference and editor installation are MS6 DX work; MS2 pins
the alias/export surface without pretending to provide types it cannot prove.

## 5. Failure and integrity rules

Compilation uses the existing source-located WOML diagnostic envelope. It
rejects build failures, missing output pairs, invalid or escaping source maps,
source-content digest mismatches, source changes during the build, and absolute
path leakage. Artifacts are bounded by the existing MS1 source limits; runtime
frame/cache limits remain MS3–MS4 work.

The package root hash covers the complete Model v9, source graph, artifact
paths/kinds/media types/digests/content, compiler identity, Bun compatibility
version, and permission inputs. Changing any executable byte changes the root
hash.
