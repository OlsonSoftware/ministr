import { renderHook, waitFor, act } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useCachedQuery, __clearQueryCache, invalidateCorpus } from "./useCachedQuery";

beforeEach(() => __clearQueryCache());

describe("useCachedQuery", () => {
  it("fetches on a miss, serves cache on remount, and refresh() busts it", async () => {
    let calls = 0;
    const fetcher = () => {
      calls += 1;
      return Promise.resolve([calls]);
    };

    // First mount — a cache miss → one fetch.
    const first = renderHook(() => useCachedQuery<number[]>("c1", "q", fetcher, []));
    await waitFor(() => expect(first.result.current.loading).toBe(false));
    expect(calls).toBe(1);
    expect(first.result.current.data).toEqual([1]);
    first.unmount();

    // Remount with the same corpus+key — served from cache, NO new fetch.
    const second = renderHook(() => useCachedQuery<number[]>("c1", "q", fetcher, []));
    await waitFor(() => expect(second.result.current.loading).toBe(false));
    expect(calls).toBe(1); // still 1 — the redundant refetch is gone
    expect(second.result.current.data).toEqual([1]);

    // Explicit refresh forces a fresh fetch (the re-run affordance).
    // Wait on the DATA, not `calls`: the fetcher bumps `calls` synchronously
    // the instant it's invoked, so a waitFor on `calls` can win the race
    // before the refetched data has flushed to state — the old intermittent
    // "[1] to equal [2]" flake. Gating on data waits for the full update.
    act(() => second.result.current.refresh());
    await waitFor(() => expect(second.result.current.data).toEqual([2]));
    expect(calls).toBe(2);
  });

  it("keys by corpus — a different corpus is a separate entry", async () => {
    let calls = 0;
    const fetcher = () => {
      calls += 1;
      return Promise.resolve([calls]);
    };

    const a = renderHook(() => useCachedQuery<number[]>("cA", "q", fetcher, []));
    await waitFor(() => expect(a.result.current.loading).toBe(false));
    const b = renderHook(() => useCachedQuery<number[]>("cB", "q", fetcher, []));
    await waitFor(() => expect(b.result.current.loading).toBe(false));

    expect(calls).toBe(2); // one miss per corpus
  });

  it("invalidateCorpus drops cached entries so the next mount refetches", async () => {
    let calls = 0;
    const fetcher = () => {
      calls += 1;
      return Promise.resolve([calls]);
    };

    const first = renderHook(() => useCachedQuery<number[]>("c1", "q", fetcher, []));
    await waitFor(() => expect(first.result.current.loading).toBe(false));
    expect(calls).toBe(1);
    first.unmount();

    invalidateCorpus("c1");

    const second = renderHook(() => useCachedQuery<number[]>("c1", "q", fetcher, []));
    await waitFor(() => expect(second.result.current.loading).toBe(false));
    expect(calls).toBe(2); // cache was invalidated → refetch
  });
});
