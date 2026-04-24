import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import { Provider } from '@/components/provider';
import './global.css';

const inter = Inter({
  subsets: ['latin'],
});

export const metadata: Metadata = {
  metadataBase: new URL('https://ministr.ai'),
  // `default` applies to the landing; the `template` wraps every
  // docs page so the tab title reads e.g. "Architecture — ministr"
  // without each MDX file having to spell it out.
  title: {
    default: 'ministr — a local index for AI coding agents',
    template: '%s — ministr',
  },
  description:
    'ministr stops Claude Code, Cursor, and Copilot from re-grepping and re-reading your code. A local MCP server that ships the exact section your agent needs — once — then tracks what it has and skips it on the next turn.',
  applicationName: 'ministr',
  authors: [{ name: 'Alrik Olson' }],
  keywords: [
    'MCP',
    'Model Context Protocol',
    'Claude Code',
    'Cursor',
    'Copilot',
    'AI coding agent',
    'semantic code search',
    'context cache',
    'local-first',
    'Rust',
  ],
  // Site is still pre-launch; keep it out of search indexes for now.
  // Flipping to `index: true` + dropping `public/robots.txt` is a
  // one-line change when we're ready.
  robots: { index: false, follow: false },
  openGraph: {
    type: 'website',
    siteName: 'ministr',
    title: 'ministr — a local index for AI coding agents',
    description:
      'A local MCP server that ships the exact section your agent needs — once — then tracks what it has and skips it on the next turn.',
    url: 'https://ministr.ai',
    // No image set yet; Fumadocs generates per-doc OG at /og/docs/<slug>.
    // The landing falls back to no-image until a real asset exists —
    // better than shipping a generic purple-gradient placeholder.
  },
  twitter: {
    card: 'summary',
    title: 'ministr — a local index for AI coding agents',
    description:
      'Stop your agent from re-reading the same files. Local MCP server, no API calls.',
  },
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
