import Link from 'next/link';

export default function MarketingLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="ministr-v2">
      <nav className="v2-nav">
        <Link href="/" className="v2-brand">
          <svg className="v2-logo" viewBox="0 0 926 926" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <defs>
              <linearGradient id="ministrLogoA" x1="1098.5" y1="-174" x2="-173.5" y2="1092" gradientUnits="userSpaceOnUse">
                <stop stopColor="#F8AC18"/>
                <stop offset="1" stopColor="#FF9900"/>
              </linearGradient>
            </defs>
            <path fillRule="evenodd" clipRule="evenodd" d="M926 926H0V0H926V926ZM241 241V685H685V241H241Z" fill="url(#ministrLogoA)"/>
          </svg>
          <span>ministr<span className="v2-dot">.</span></span>
        </Link>
        <div className="v2-nav-links">
          <Link href="/install">install</Link>
          <Link href="/pricing">pricing</Link>
          <Link href="/docs">docs</Link>
          <a href="https://github.com/OlsonSoftware/ministr" target="_blank" rel="noopener noreferrer">github</a>
        </div>
      </nav>
      {children}
    </div>
  );
}
