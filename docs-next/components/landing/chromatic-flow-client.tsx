'use client';

import dynamic from 'next/dynamic';

/**
 * Client-only wrapper for ChromaticFlow. Next 16 forbids `ssr: false`
 * in Server Components, so the dynamic import happens inside this
 * Client Component boundary.
 */
const ChromaticFlow = dynamic(
  () => import('@/components/landing/chromatic-flow').then((m) => m.ChromaticFlow),
  { ssr: false },
);

export function ChromaticFlowClient() {
  return <ChromaticFlow />;
}
