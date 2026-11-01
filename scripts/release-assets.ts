export interface ReleaseAsset {
  id: number;
  name: string;
  size: number;
}

export interface UpdateManifest {
  version: string;
  platforms: Record<string, { url: string; signature: string }>;
}

export function checkReleaseAssets(
  tag: string,
  assets: ReleaseAsset[],
  manifest: UpdateManifest,
  signatures: Record<string, string>,
) {
  const version = tag.replace(/^v/, '');
  if (!/^v\d+\.\d+\.\d+$/.test(tag) || manifest.version.replace(/^v/, '') !== version) {
    throw new Error('更新清单版本与发布 tag 不一致');
  }
  const installers = [`FlairBloom_${version}_x64.msi`, `FlairBloom_${version}_x64-setup.exe`];
  const required = ['latest.json', ...installers, ...installers.map((name) => `${name}.sig`)];
  for (const name of required) {
    if (!assets.some((asset) => asset.name === name && asset.size > 0)) {
      throw new Error(`发布产物缺失或为空：${name}`);
    }
  }
  const expected = {
    'windows-x86_64': installers[0],
    'windows-x86_64-msi': installers[0],
    'windows-x86_64-nsis': installers[1],
  };
  for (const [platform, name] of Object.entries(expected)) {
    const entry = manifest.platforms[platform];
    const expectedUrl = `https://github.com/x-wink/flair-bloom/releases/download/${tag}/${name}`;
    if (
      !entry ||
      entry.url !== expectedUrl ||
      !signatures[name]?.trim() ||
      entry.signature.trim() !== signatures[name].trim()
    ) {
      throw new Error(`更新清单的下载地址或签名不匹配：${platform}`);
    }
  }
  return assets
    .filter((asset) => asset.name.startsWith('FlairBloom_'))
    .map((asset) => ({
      id: asset.id,
      label: asset.name.replace(/^FlairBloom_/, '气质花_'),
    }));
}
