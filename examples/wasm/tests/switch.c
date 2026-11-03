__attribute__((import_module("env"), import_name("print_i32"))) void print_i32(int);
__attribute__((import_module("env"), import_name("putchar"))) void putch(int);

__attribute__((noinline)) int classify(const char *f) {
  int n = 0;
  for (; *f; f++) {
    if (*f != '%') { putch(*f); continue; }
    f++;
    switch (*f) {
      case 's': putch('S'); n++; break;
      case 'd': putch('D'); n++; break;
      case 'x': putch('X'); n++; break;
      case 'c': putch('C'); n++; break;
      case '%': putch('%'); break;
      default: putch('?'); break;
    }
  }
  putch('\n');
  return n;
}

__attribute__((noinline)) int sel(int a, int b, int c) { return c ? a : b; }
__attribute__((noinline)) unsigned char lower(unsigned char c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }

__attribute__((export_name("main"))) int main(void) {
  print_i32(classify(">%s %s\n %d%%x %c"));
  print_i32(sel(1, 2, 0));
  print_i32(sel(1, 2, 5));
  print_i32(lower('Q'));
  print_i32(lower('q'));
  return 0;
}
