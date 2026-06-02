// Sorted-array search utilities: locating an element and the classic
// lower-/upper-bound boundary queries. Part of the code-heavy evaluation
// corpus (eval/corpus-code) used to benchmark embedders on text-to-code
// retrieval.

#include <cstddef>
#include <vector>

namespace search {

// Return the index of `target` in the ascending-sorted `data`, or -1 if it is
// absent. Halves the search interval each step, so it runs in O(log n).
long long binary_search(const std::vector<int>& data, int target) {
    long long lo = 0;
    long long hi = static_cast<long long>(data.size()) - 1;
    while (lo <= hi) {
        long long mid = lo + (hi - lo) / 2;
        if (data[mid] == target) {
            return mid;
        }
        if (data[mid] < target) {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    return -1;
}

// Index of the first element not less than `target` (the leftmost insertion
// point that keeps `data` sorted). Equivalent to C++ std::lower_bound.
std::size_t lower_bound(const std::vector<int>& data, int target) {
    std::size_t lo = 0;
    std::size_t hi = data.size();
    while (lo < hi) {
        std::size_t mid = lo + (hi - lo) / 2;
        if (data[mid] < target) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    return lo;
}

// Index of the first element strictly greater than `target`. The half-open gap
// between lower_bound and upper_bound is the range of entries equal to target,
// which is how you count duplicates in a sorted array.
std::size_t upper_bound(const std::vector<int>& data, int target) {
    std::size_t lo = 0;
    std::size_t hi = data.size();
    while (lo < hi) {
        std::size_t mid = lo + (hi - lo) / 2;
        if (data[mid] <= target) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    return lo;
}

}  // namespace search
