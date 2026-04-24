import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import { Provider } from '@/components/provider';
import './global.css';

const inter = Inter({
  subsets: ['latin'],
});

// Canonical host for all OpenGraph / Twitter / absolute-URL resolution.
// Drives how Next.js expands relative `openGraph.images`, canonical tags,
// and the sitemap absolute URLs.
export const metadata: Metadata = {
  metadataBase: new URL('https://ministr.ai'),
  robots: { index: false, follow: false },
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html lang="en" className={inter.className} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
