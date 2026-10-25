export function npmExecutableForPlatform(platform: string): string {
  return platform === 'win32' ? 'npm.cmd' : 'npm';
}
