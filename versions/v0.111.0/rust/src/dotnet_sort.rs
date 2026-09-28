//! Swap-for-swap port of .NET 9 `GenericArraySortHelper<T>.IntroSort`.

const INTROSORT_SIZE_THRESHOLD: usize = 16;

pub fn dotnet_list_sort_by_key<T, K, F>(items: &[T], mut key: F) -> Vec<T>
where
    T: Clone,
    K: Ord,
    F: FnMut(&T) -> K,
{
    let mut keyed: Vec<(K, T)> = items.iter().map(|item| (key(item), item.clone())).collect();
    let n = keyed.len();
    if n > 1 {
        let depth = 2 * ((usize::BITS - n.leading_zeros()) as usize);
        intro_sort(&mut keyed, 0, n, depth);
    }
    keyed.into_iter().map(|(_, item)| item).collect()
}

fn swap_if_greater<K: Ord, T>(items: &mut [(K, T)], i: usize, j: usize) {
    if items[i].0 > items[j].0 {
        items.swap(i, j);
    }
}

fn intro_sort<K: Ord, T>(items: &mut [(K, T)], lo: usize, size: usize, mut depth: usize) {
    let mut partition_size = size;
    while partition_size > 1 {
        if partition_size <= INTROSORT_SIZE_THRESHOLD {
            match partition_size {
                2 => swap_if_greater(items, lo, lo + 1),
                3 => {
                    swap_if_greater(items, lo, lo + 1);
                    swap_if_greater(items, lo, lo + 2);
                    swap_if_greater(items, lo + 1, lo + 2);
                }
                _ => insertion_sort(items, lo, partition_size),
            }
            return;
        }
        if depth == 0 {
            heap_sort(items, lo, partition_size);
            return;
        }
        depth -= 1;
        let pivot = pick_pivot_and_partition(items, lo, partition_size);
        intro_sort(items, lo + pivot + 1, partition_size - (pivot + 1), depth);
        partition_size = pivot;
    }
}

fn pick_pivot_and_partition<K: Ord, T>(items: &mut [(K, T)], lo: usize, n: usize) -> usize {
    let zero = lo;
    let last = lo + n - 1;
    let middle = lo + ((n - 1) >> 1);
    swap_if_greater(items, zero, middle);
    swap_if_greater(items, zero, last);
    swap_if_greater(items, middle, last);
    let next_to_last = lo + n - 2;
    items.swap(middle, next_to_last);
    let mut left = zero;
    let mut right = next_to_last;
    while left < right {
        while left < next_to_last {
            left += 1;
            if items[left].0 >= items[next_to_last].0 {
                break;
            }
        }
        while right > zero {
            right -= 1;
            if items[right].0 <= items[next_to_last].0 {
                break;
            }
        }
        if left >= right {
            break;
        }
        items.swap(left, right);
    }
    if left != next_to_last {
        items.swap(left, next_to_last);
    }
    left - zero
}

fn insertion_sort<K: Ord, T>(items: &mut [(K, T)], lo: usize, size: usize) {
    for i in 0..size - 1 {
        let mut j = i;
        while j < size && items[lo + j + 1].0 < items[lo + j].0 {
            items.swap(lo + j, lo + j + 1);
            if j == 0 {
                break;
            }
            j -= 1;
        }
    }
}

fn heap_sort<K: Ord, T>(items: &mut [(K, T)], lo: usize, n: usize) {
    for i in (1..=n >> 1).rev() {
        down_heap(items, lo, i, n);
    }
    for i in (2..=n).rev() {
        items.swap(lo, lo + i - 1);
        down_heap(items, lo, 1, i - 1);
    }
}

fn down_heap<K: Ord, T>(items: &mut [(K, T)], lo: usize, mut i: usize, n: usize) {
    while i <= n >> 1 {
        let mut child = 2 * i;
        if child < n && items[lo + child - 1].0 < items[lo + child].0 {
            child += 1;
        }
        if items[lo + i - 1].0 >= items[lo + child - 1].0 {
            break;
        }
        items.swap(lo + i - 1, lo + child - 1);
        i = child;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_equal_keys_are_stable() {
        let input = vec![(1, "a"), (0, "x"), (1, "b"), (1, "c")];
        assert_eq!(
            dotnet_list_sort_by_key(&input, |value| value.0),
            vec![(0, "x"), (1, "a"), (1, "b"), (1, "c")]
        );
    }

    #[test]
    fn large_tie_reordering_matches_python_dotnet9_corpus() {
        let input: Vec<(i32, usize)> =
            vec![2, 1, 1, 0, 2, 1, 0, 1, 2, 0, 1, 1, 2, 0, 0, 1, 2, 1, 0, 2]
                .into_iter()
                .enumerate()
                .map(|(index, key)| (key, index))
                .collect();
        let output = dotnet_list_sort_by_key(&input, |value| value.0);
        let identities: Vec<usize> = output.into_iter().map(|value| value.1).collect();
        assert_eq!(
            identities,
            vec![
                9, 3, 6, 18, 13, 14, 1, 2, 17, 5, 7, 10, 11, 15, 16, 0, 8, 4, 12, 19
            ]
        );
    }
}
