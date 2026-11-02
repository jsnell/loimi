var list = { value: 1, next: { value: 2, next: null } };
var node = list;
while (true) {
  console.log("value", node.value);
  node = node.next;
  if (node.value > 100) { break; }
}
