import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import Link from 'next/link';

function MinistrLogo() {
  return (
    <Link href="/" style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', textDecoration: 'none', color: 'inherit' }}>
      <svg width="24" height="24" viewBox="0 0 926 926" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <defs>
          <linearGradient id="ministrNavLogo" x1="1098.5" y1="-174" x2="-173.5" y2="1092" gradientUnits="userSpaceOnUse">
            <stop stopColor="#F8AC18"/>
            <stop offset="1" stopColor="#FF9900"/>
          </linearGradient>
        </defs>
        <path fillRule="evenodd" clipRule="evenodd" d="M926 926H0V0H926V926ZM241 241V685H685V241H241Z" fill="url(#ministrNavLogo)"/>
      </svg>
      <span style={{ fontWeight: 600, letterSpacing: '-0.02em' }}>ministr</span>
    </Link>
  );
}

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: <MinistrLogo />,
    },
    githubUrl: 'https://github.com/OlsonSoftware/ministr',
    links: [
      { text: 'Install', url: '/install' },
      { text: 'Pricing', url: '/pricing' },
    ],
  };
}
