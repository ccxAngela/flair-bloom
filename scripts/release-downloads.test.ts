import assert from 'node:assert/strict';
import test from 'node:test';

// Nuxt 为该模块注入的只读站点地址；其余解析逻辑使用生产实现。
Object.assign(globalThis, { REPOSITORY: 'https://example.test/flair-bloom' });
const { toReleaseEntries, GITHUB_PROXY } = await import('../apps/site/app/composables/useReleases.ts');

test('中文安装包与签名同列时，官网仅提供两个有效安装器且保留编码下载地址', () => {
  const names = ['气质花_0.7.1_x64-setup.exe', '气质花_0.7.1_x64.msi'];
  const urls = names.map((name) => `https://example.test/download/v0.7.1/${encodeURIComponent(name)}`);
  const [release] = toReleaseEntries([{
    tag_name: 'v0.7.1', name: '气质花', body: '', draft: false, prerelease: false,
    published_at: '2026-11-01T00:00:00Z', html_url: 'https://example.test/release/v0.7.1',
    assets: [...names, ...names.map((name) => `${name}.sig`), 'latest.json'].map((name, i) => ({
      name, size: 100, browser_download_url: urls[i] ?? 'https://example.test/metadata',
    })),
  }]);
  assert.deepEqual(release.assets.map((asset) => [asset.kind, asset.name, asset.githubUrl, asset.url]), [
    ['nsis', names[0], urls[0], `${GITHUB_PROXY}${urls[0]}`],
    ['msi', names[1], urls[1], `${GITHUB_PROXY}${urls[1]}`],
  ]);
});

test('中文命名不影响旧英文安装包识别，草稿发布仍不展示', () => {
  const base = {
    tag_name: 'v0.7.0', name: 'FlairBloom', body: '', draft: false, prerelease: false,
    published_at: '2026-10-01T00:00:00Z', html_url: 'https://example.test/release/v0.7.0',
    assets: [{ name: 'FlairBloom_0.7.0_x64-setup.exe', size: 100, browser_download_url: 'https://example.test/legacy' }],
  };
  const releases = toReleaseEntries([{ ...base, draft: true, tag_name: 'v0.7.1' }, base]);
  assert.equal(releases.length, 1);
  assert.equal(releases[0].assets[0].kind, 'nsis');
  assert.equal(releases[0].assets[0].name, 'FlairBloom_0.7.0_x64-setup.exe');
});
