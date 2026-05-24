import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { appName } from './shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      // JSX supported
      title: appName,
    },
    // No `githubUrl` — ministr is a closed product; the repo is private.
    // Showing a GitHub icon in the nav primes visitors to expect OSS.
  };
}
