import { useEffect, useState } from "react";

/** `value`, settled for `delay` ms (for search-as-you-type). */
export function useDebounced<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => {
      setSettled(value);
    }, delay);
    return () => {
      clearTimeout(t);
    };
  }, [value, delay]);
  return settled;
}
