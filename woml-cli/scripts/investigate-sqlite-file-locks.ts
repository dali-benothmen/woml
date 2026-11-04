#!/usr/bin/env bun
// Diagnostic only: reproduce the POSIX descriptor-close hazard in a disposable
// database. Never run against workflow state and never use as a release gate.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { closeSync, fstatSync, openSync, statSync } from 'node:fs';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Database } from 'bun:sqlite';

if (process.argv[2] === '--writer') {
  const database = new Database(process.argv[3]!);
  try {
    database.exec('PRAGMA busy_timeout=0;');
    database.exec('BEGIN IMMEDIATE;');
    database.exec('ROLLBACK;');
    console.log('writer-acquired');
  } catch (error) {
    if (!/database is locked/i.test(String(error))) throw error;
    console.log('writer-blocked');
  } finally { database.close(); }
} else {
  assert.equal(process.argv.length, 2, 'No database path accepted: this probe always creates disposable state');
  assert.equal(process.platform, 'linux', 'This diagnosis currently covers Linux POSIX locks only');
  const directory = await mkdtemp(join(tmpdir(), 'woml-sqlite-file-locks-'));
  const path = join(directory, 'diagnosis.sqlite');
  const database = new Database(path, { create: true });
  const writer = () => {
    const result = Bun.spawnSync([process.execPath, import.meta.path, '--writer', path], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(result.exitCode, 0, result.stderr.toString());
    return result.stdout.toString().trim();
  };
  try {
    database.exec('PRAGMA journal_mode=DELETE;');
    database.exec('CREATE TABLE diagnosis(value INTEGER);');
    database.exec('BEGIN IMMEDIATE;');
    const before = writer();
    statSync(path); // Path metadata does not create/close another database FD.
    const afterMetadata = writer();
    const descriptor = openSync(path, 'r');
    try { fstatSync(descriptor); } finally { closeSync(descriptor); }
    const afterDescriptorClose = writer();
    const guard = await readFile(resolve(import.meta.dir, '../../core/woml-engine/src/durable/prepared.rs'), 'utf8');
    const report = { profile: 'woml.sqlite-descriptor-lock-diagnosis/v1', directory,
      before, afterMetadata, afterDescriptorClose,
      unsafeDescriptorCloseReproduced: before === 'writer-blocked' && afterMetadata === 'writer-blocked' && afterDescriptorClose === 'writer-acquired',
      guardSourceSha256: createHash('sha256').update(guard).digest('hex'),
      currentGuardContainsTemporaryPinOpen: /pin_file\(&self\.0\.canonical\)\s*\.and_then\(\|file\| file_identity\(&file\)\)/u.test(guard),
      scope: 'OS-level reproduction of the guard operation sequence using Bun SQLite. Not direct invocation of the private Rust guard, not attribution of every I/O failure, and not a fix.' };
    await Bun.write(join(directory, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    assert.equal(report.unsafeDescriptorCloseReproduced, true, 'Expected POSIX hazard was not reproduced; do not infer the cause');
  } finally { database.exec('ROLLBACK;'); database.close(); }
}
