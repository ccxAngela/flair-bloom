import assert from 'node:assert/strict';
import test from 'node:test';
import { checkReleaseAssets, type UpdateManifest } from './release-assets.ts';

function fixture() {
  const msi = 'FlairBloom_0.8.0_x64.msi';
  const exe = 'FlairBloom_0.8.0_x64-setup.exe';
  const names = [msi, exe, `${msi}.sig`, `${exe}.sig`, 'latest.json'];
  const signatures: Record<string, string> = { [msi]: 'msi-signature\n', [exe]: 'exe-signature\n' };
  const entry = (name: string) => ({
    url: `https://github.com/x-wink/flair-bloom/releases/download/v0.8.0/${name}`,
    signature: signatures[name].trim(),
  });
  return {
    assets: names.map((name, id) => ({ id, name, size: 100 })),
    manifest: {
      version: '0.8.0',
      platforms: {
        'windows-x86_64': entry(msi),
        'windows-x86_64-msi': entry(msi),
        'windows-x86_64-nsis': entry(exe),
      },
    } satisfies UpdateManifest,
    signatures,
  };
}

test('ASCII 下载地址与中文显示名称独立，两个安装器及签名必须配齐', () => {
  const { assets, manifest, signatures } = fixture();
  const labels = checkReleaseAssets('v0.8.0', assets, manifest, signatures);
  assert.deepEqual(
    labels.map((asset) => asset.label),
    [
      '气质花_0.8.0_x64.msi',
      '气质花_0.8.0_x64-setup.exe',
      '气质花_0.8.0_x64.msi.sig',
      '气质花_0.8.0_x64-setup.exe.sig',
    ],
  );
});

test('构建成功但缺少 latest.json、安装包或签名时阻止发布', () => {
  const { assets, manifest, signatures } = fixture();
  for (const omitted of assets)
    assert.throws(
      () =>
        checkReleaseAssets(
          'v0.8.0',
          assets.filter((asset) => asset !== omitted),
          manifest,
          signatures,
        ),
      /缺失/,
    );
});

test('平台过滤中文后产生的实际名称不能冒充预期产物', () => {
  const { assets, manifest, signatures } = fixture();
  assets[0].name = '_0.8.0_x64.msi';
  assert.throws(() => checkReleaseAssets('v0.8.0', assets, manifest, signatures), /缺失/);
});

test('版本、下载地址、平台或签名不匹配时阻止发布', () => {
  for (const problem of ['version', 'url', 'signature', 'platform']) {
    const { assets, manifest, signatures } = fixture();
    if (problem === 'version') manifest.version = '0.7.1';
    if (problem === 'url')
      manifest.platforms['windows-x86_64'].url = 'https://example.test/wrong.msi';
    if (problem === 'signature') manifest.platforms['windows-x86_64'].signature = 'wrong';
    if (problem === 'platform')
      delete (manifest.platforms as UpdateManifest['platforms'])['windows-x86_64'];
    assert.throws(
      () => checkReleaseAssets('v0.8.0', assets, manifest, signatures),
      /不一致|不匹配/,
    );
  }
});
