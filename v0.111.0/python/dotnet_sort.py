"""
Exact port of .NET 9's List<T>.Sort() — the sort inside the game's
reshuffle (`ListExtensions.StableShuffle` = source.ToList() -> copy.Sort()
-> write back -> UnstableShuffle), needed because CardModel.CompareTo
returns 0 on full (id, upgrade) ties and the game's tie order is therefore
whatever this exact algorithm does with the discard's live order.

Provenance (IL-read 2026-07-10, this machine's live install):
  - The build is Godot .NET, self-contained net9.0 (sts2.runtimeconfig.json:
    Microsoft.NETCore.App 9.0.7). The BCL is the shipped
    System.Private.CoreLib.dll (assembly version 9.0.0.0) — NOT the old
    .NET Framework referencesource introsort (whose depth limit reads the
    backing array's capacity); the span-based one below sorts exactly
    Count elements.
  - CardModel implements IComparable<AbstractModel> (via AbstractModel);
    contravariance makes IComparable<CardModel>.IsAssignableFrom(CardModel)
    true, so ArraySortHelper<CardModel>.Default is
    GenericArraySortHelper<CardModel>, and List.Sort()'s null comparer
    takes the `comparer == null || comparer == Comparer<T>.Default` branch:
    IntroSort(span, 2 * (Log2(n) + 1)) with CompareTo-based
    LessThan/GreaterThan (the double/float NaN pre-pass is skipped for
    reference types).
  - Every method below is a swap-for-swap transcription of the dumped IL of
    GenericArraySortHelper`1 (RVAs 0x980334 IntroSort, 0x9803f8
    PickPivotAndPartition, 0x980654 InsertionSort, 0x98053c HeapSort,
    0x980594 DownHeap). The null-element branches of the IL are dropped:
    a card pile never contains null (CardPile.AddInternal throws on
    duplicates and nulls never enter).

Behavioral notes that matter for the solver:
  - partitions <= 16 elements are insertion-sorted => equal keys keep
    their input order (stable);
  - partitions > 16 go through median-of-three quicksort => equal keys
    can be reordered, as a pure function of the whole input arrangement;
  - the depth-limit heapsort fallback is ported for completeness but is
    unreachable at deck sizes (2*(log2 n+1) levels with each level
    shedding at least the pivot).
"""

from __future__ import annotations

INTROSORT_SIZE_THRESHOLD = 16   # Array.IntrosortSizeThreshold


def dotnet_list_sort(items, key):
    """Return a new list: `items` sorted exactly as .NET 9 List<T>.Sort()
    would with CompareTo == comparison of key(item)."""
    a = [(key(x), x) for x in items]
    n = len(a)
    if n > 1:
        # Sort(): IntroSort(keys, 2 * (BitOperations.Log2((uint)Length)+1))
        _intro_sort(a, 0, n, 2 * ((n.bit_length() - 1) + 1))
    return [x for _, x in a]


def _swap_if_greater(a, i, j):
    if a[i][0] > a[j][0]:
        a[i], a[j] = a[j], a[i]


def _intro_sort(a, lo, size, depth_limit):
    # IntroSort(Span keys, int depthLimit): while-loop on the left part,
    # recursion on the right part; the pivot at p is excluded from both.
    partition_size = size
    while partition_size > 1:
        if partition_size <= INTROSORT_SIZE_THRESHOLD:
            if partition_size == 2:
                _swap_if_greater(a, lo, lo + 1)
                return
            if partition_size == 3:
                _swap_if_greater(a, lo, lo + 1)
                _swap_if_greater(a, lo, lo + 2)
                _swap_if_greater(a, lo + 1, lo + 2)
                return
            _insertion_sort(a, lo, partition_size)
            return
        if depth_limit == 0:
            _heap_sort(a, lo, partition_size)
            return
        depth_limit -= 1
        p = _pick_pivot_and_partition(a, lo, partition_size)
        _intro_sort(a, lo + p + 1, partition_size - (p + 1), depth_limit)
        partition_size = p


def _pick_pivot_and_partition(a, lo, n):
    # median-of-three over (0, (n-1)>>1, n-1); pivot parked at n-2;
    # pre-increment/pre-decrement scans with the IL's bounds guards.
    zero, last = lo, lo + n - 1
    middle = lo + ((n - 1) >> 1)
    _swap_if_greater(a, zero, middle)
    _swap_if_greater(a, zero, last)
    _swap_if_greater(a, middle, last)
    next_to_last = lo + n - 2
    pivot = a[middle]
    a[middle], a[next_to_last] = a[next_to_last], a[middle]
    left, right = zero, next_to_last
    while left < right:
        while left < next_to_last:
            left += 1
            if not pivot[0] > a[left][0]:
                break
        while right > zero:
            right -= 1
            if not pivot[0] < a[right][0]:
                break
        if left >= right:
            break
        a[left], a[right] = a[right], a[left]
    if left != next_to_last:
        a[left], a[next_to_last] = a[next_to_last], a[left]
    return left - zero


def _insertion_sort(a, lo, size):
    # stable: shifts only while strictly less-than
    for i in range(size - 1):
        t = a[lo + i + 1]
        j = i
        while j >= 0 and t[0] < a[lo + j][0]:
            a[lo + j + 1] = a[lo + j]
            j -= 1
        a[lo + j + 1] = t


def _heap_sort(a, lo, n):
    for i in range(n >> 1, 0, -1):
        _down_heap(a, lo, i, n)
    for i in range(n, 1, -1):
        a[lo], a[lo + i - 1] = a[lo + i - 1], a[lo]
        _down_heap(a, lo, 1, i - 1)


def _down_heap(a, lo, i, n):
    d = a[lo + i - 1]
    while i <= n >> 1:
        child = 2 * i
        if child < n and a[lo + child - 1][0] < a[lo + child][0]:
            child += 1
        if not d[0] < a[lo + child - 1][0]:
            break
        a[lo + i - 1] = a[lo + child - 1]
        i = child
    a[lo + i - 1] = d
