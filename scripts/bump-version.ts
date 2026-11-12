import { readFileSync, writeFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { formatReleaseDate } from './release-date.ts';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const raw = process.argv[2];
if (!raw) {
  console.error('用法：pnpm bump-version <X.Y.Z>');
  process.exit(1);
}
const version = raw.replace(/^v/, '');
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`版本号格式错误：${raw}，需为 X.Y.Z`);
  process.exit(1);
}

interface FileUpdater {
  path: string;
  currentVersion(content: string): string | undefined;
  update(content: string): string;
}

const today = formatReleaseDate();

const files: FileUpdater[] = [
  {
    path: 'apps/main/package.json',
    currentVersion: (c) => (JSON.parse(c) as { version: string }).version,
    update(c) {
      const obj = JSON.parse(c) as Record<string, unknown>;
      obj.version = version;
      return JSON.stringify(obj, null, 2) + '\n';
    },
  },
  {
    path: 'apps/main/src-tauri/tauri.conf.json',
    currentVersion: (c) => (JSON.parse(c) as { version: string }).version,
    update(c) {
      const obj = JSON.parse(c) as Record<string, unknown>;
      obj.version = version;
      return JSON.stringify(obj, null, 2) + '\n';
    },
  },
  {
    path: 'Cargo.toml',
    currentVersion: (c) => c.match(/^version = "(.*)"/m)?.[1],
    update: (c) => c.replace(/^version = ".*"$/m, `version = "${version}"`),
  },
  {
    path: 'CHANGELOG.md',
    currentVersion: (c) => (c.includes(`## [${version}]`) ? version : undefined),
    update(c) {
      const UNRELEASED = '## [Unreleased]';
      const newSection = `${UNRELEASED}\n\n## [${version}] - ${today}`;
      return c.includes(UNRELEASED)
        ? c.replace(UNRELEASED, newSection)
        : c.replace(/^(## \[)/, `## [${version}] - ${today}\n\n$1`);
    },
  },
];

for (const { path, currentVersion, update } of files) {
  const abs = resolve(root, path);
  const before = readFileSync(abs, 'utf8');
  if (currentVersion(before) === version) {
    console.log(`  跳过 ${path}（已是 v${version}）`);
    continue;
  }
  writeFileSync(abs, update(before));
  console.log(`  已更新 ${path}`);
}

validateCargoWorkspaceMembers(version);

console.log(`\n版本号已升至 v${version}`);
console.log(`提示：在 CHANGELOG.md 的 [${version}] 节填写本次更新内容后再提交`);
console.log(`提交：git add -p && git commit -m "chore(release): bump version to ${version}"`);

function validateCargoWorkspaceMembers(expectedVersion: string) {
  const workspaceToml = readFileSync(resolve(root, 'Cargo.toml'), 'utf8');
  const membersMatch = workspaceToml.match(/members\s*=\s*\[([\s\S]*?)\]/m);
  if (!membersMatch) {
    throw new Error('Cargo.toml 未找到 workspace.members');
  }

  const members = Array.from(membersMatch[1].matchAll(/"([^"]+)"/g)).map((m) => m[1]);
  const invalid: string[] = [];
  for (const member of members) {
    const cargoTomlPath = resolve(root, member, 'Cargo.toml');
    const cargoToml = readFileSync(cargoTomlPath, 'utf8');
    const usesWorkspaceVersion = /^\s*version\.workspace\s*=\s*true\s*$/m.test(cargoToml);
    const explicitVersion = cargoToml.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m)?.[1];
    if (!usesWorkspaceVersion && explicitVersion !== expectedVersion) {
      invalid.push(`${member}/Cargo.toml`);
    }
  }

  if (invalid.length > 0) {
    console.error('\n以下 Rust workspace 成员未跟随 workspace 版本，请同步更新：');
    for (const path of invalid) {
      console.error(`  - ${path}`);
    }
    process.exit(1);
  }
  console.log('  已校验 Rust workspace 成员版本');
}
