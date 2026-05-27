import type { Metadata } from 'next';
import { InstallClient } from '@/components/landing/install-client';

export const metadata: Metadata = {
  title: 'Install ministr',
  description:
    'Download ministr — desktop installers for macOS, Windows, and Linux, plus a one-line CLI install for any platform.',
  alternates: {
    canonical: '/install',
  },
};

export default function InstallPage() {
  return <InstallClient />;
}
