import { execFileSync } from 'node:child_process';
import { checkReleaseAssets, type ReleaseAsset, type UpdateManifest } from './release-assets.ts';

const tag = process.argv[2];
if (!tag || !/^v\d+\.\d+\.\d+$/.test(tag)) throw new Error('需传入 vX.Y.Z 发布 tag');
const repo = 'repos/x-wink/flair-bloom';
function gh(...args: string[]) {
  return execFileSync('gh', args, { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 });
}
const releaseId = (
  JSON.parse(gh('release', 'view', tag, '--json', 'databaseId')) as { databaseId: number }
).databaseId;
const release = JSON.parse(gh('api', `${repo}/releases/${releaseId}`)) as {
  tag_name: string;
  assets: ReleaseAsset[];
};
if (release.tag_name !== tag) throw new Error('发布记录与 tag 不一致');
function readAsset(name: string) {
  const asset = release.assets.find((item) => item.name === name && item.size > 0);
  if (!asset) throw new Error(`发布产物缺失或为空：${name}`);
  return gh('api', `${repo}/releases/assets/${asset.id}`, '-H', 'Accept: application/octet-stream');
}
const manifest = JSON.parse(readAsset('latest.json')) as UpdateManifest;
const signatures = Object.fromEntries(
  release.assets
    .filter((asset) => asset.name.endsWith('.sig'))
    .map((asset) => [asset.name.slice(0, -4), readAsset(asset.name)]),
);
const labels = checkReleaseAssets(tag, release.assets, manifest, signatures);
// GitHub 会过滤下载文件名中的中文，显示名称使用独立 label，更新地址保持 ASCII。
for (const asset of labels) {
  gh(
    'api',
    '--method',
    'PATCH',
    `${repo}/releases/assets/${asset.id}`,
    '-f',
    `label=${asset.label}`,
  );
}
process.stdout.write(
  `发布产物、更新清单与签名配对检查通过，${labels.length} 个文件已设置中文显示名称\n`,
);
