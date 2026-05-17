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
    default: 'ministr — real codebase understanding for AI coding agents',
    template: '%s — ministr',
  },
  description:
    'ministr is a code intelligence MCP server for Claude Code, Cursor, and Copilot. AST-level semantic search, symbol navigation, reference graphs, and cross-language bridge detection across ~29 languages — all local.',
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
    'code intelligence',
    'cross-language bridges',
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
    title: 'ministr — real codebase understanding for AI coding agents',
    description:
      'A local code intelligence MCP server: AST-level semantic search, symbol navigation, reference graphs, and cross-language bridge detection.',
    url: 'https://ministr.ai',
    // No image set yet; Fumadocs generates per-doc OG at /og/docs/<slug>.
    // The landing falls back to no-image until a real asset exists —
    // better than shipping a generic purple-gradient placeholder.
  },
  twitter: {
    card: 'summary',
    title: 'ministr — real codebase understanding for AI coding agents',
    description:
      'Code intelligence MCP server: semantic search, symbol navigation, and cross-language bridges. Local, no API calls.',
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
