function pow(base, exponent) {
  let x = 0;
  let result = 1;
  while (x < exponent) {
    result *= base;
    x += 1;
  }
  return result;
}

console.log(pow(2, 3));
