package corpus.code;

/**
 * Disjoint-set (union-find) data structure with path compression and union by
 * rank. Part of the code-heavy evaluation corpus (eval/corpus-code) used to
 * benchmark embedders on text-to-code retrieval.
 *
 * It tracks a partition of {0, 1, ..., n-1} into disjoint groups and answers
 * "are these two elements in the same group?" in near-constant amortized time.
 */
public final class UnionFind {
    private final int[] parent;
    private final int[] rank;
    private int componentCount;

    /** Create n singleton sets, each element its own representative. */
    public UnionFind(int n) {
        parent = new int[n];
        rank = new int[n];
        componentCount = n;
        for (int i = 0; i < n; i++) {
            parent[i] = i;
        }
    }

    /**
     * Find the representative (root) of the set containing x, compressing the
     * path so future lookups are flatter and therefore faster.
     */
    public int find(int x) {
        while (parent[x] != x) {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        return x;
    }

    /**
     * Merge the sets containing a and b, attaching the shorter tree under the
     * taller one (union by rank) to keep the structure shallow. Returns false
     * when the two elements were already connected.
     */
    public boolean union(int a, int b) {
        int ra = find(a);
        int rb = find(b);
        if (ra == rb) {
            return false;
        }
        if (rank[ra] < rank[rb]) {
            int tmp = ra;
            ra = rb;
            rb = tmp;
        }
        parent[rb] = ra;
        if (rank[ra] == rank[rb]) {
            rank[ra]++;
        }
        componentCount--;
        return true;
    }

    /** Report whether two elements currently belong to the same set. */
    public boolean connected(int a, int b) {
        return find(a) == find(b);
    }

    /** Number of disjoint groups remaining after all unions so far. */
    public int components() {
        return componentCount;
    }
}
