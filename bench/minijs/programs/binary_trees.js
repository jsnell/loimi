// binary-trees, after the Computer Language Benchmarks Game.
// Stresses allocation of many small objects and recursion.
const MAX_DEPTH = 16;

class TreeNode {
  constructor(left, right) {
    this.left = left;
    this.right = right;
  }
  check() {
    if (this.left === null) {
      return 1;
    }
    return 1 + this.left.check() + this.right.check();
  }
}

function bottomUpTree(depth) {
  if (depth > 0) {
    depth = depth - 1;
    return new TreeNode(bottomUpTree(depth), bottomUpTree(depth));
  }
  return new TreeNode(null, null);
}

function main(maxDepth) {
  const minDepth = 4;
  if (minDepth + 2 > maxDepth) {
    maxDepth = minDepth + 2;
  }
  const stretchDepth = maxDepth + 1;
  console.log("stretch tree of depth " + stretchDepth + "\t check: " + bottomUpTree(stretchDepth).check());
  const longLived = bottomUpTree(maxDepth);
  let total = 0;
  for (let depth = minDepth; depth <= maxDepth; depth += 2) {
    const iterations = 1 << (maxDepth - depth + minDepth);
    let check = 0;
    for (let i = 0; i < iterations; i++) {
      check += bottomUpTree(depth).check();
    }
    total += check;
    console.log(iterations + "\t trees of depth " + depth + "\t check: " + check);
  }
  console.log("long lived tree of depth " + maxDepth + "\t check: " + longLived.check());
  console.log("checksum " + total);
}

main(MAX_DEPTH);
