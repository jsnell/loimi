__attribute__((import_module("env"), import_name("print_i32"))) void print_i32(int);
__attribute__((import_module("env"), import_name("print_f64"))) void print_f64(double);
__attribute__((import_module("env"), import_name("putchar"))) void putch(int);
static int fib(int n) { return n < 2 ? n : fib(n-1) + fib(n-2); }
int arr[100];
typedef int (*fp)(int);
static int sq(int x) { return x*x; }
static int neg(int x) { return -x; }
fp table[2] = { sq, neg };
__attribute__((noinline)) double fsum(int n) { double s = 0; for (int i = 1; i <= n; i++) s += 1.0 / i; return s; }
__attribute__((export_name("main"))) int main(void) {
  print_i32(fib(20));
  for (int i = 0; i < 100; i++) arr[i] = i * i;
  int s = 0; for (int i = 0; i < 100; i++) s += arr[i];
  print_i32(s);
  print_f64(fsum(1000));
  volatile int k = 1;
  print_i32(table[k](7) + table[0](3));
  long long x = 1; for (int i = 0; i < 40; i++) x = x * 3 + 1;
  print_i32((int)(x % 1000000007));
  const char *msg = "hi\n"; while (*msg) putch(*msg++);
  switch (k) { case 0: print_i32(100); break; case 1: print_i32(101); break; case 2: print_i32(102); break; case 3: print_i32(103); break; case 4: print_i32(104); break; default: print_i32(-1); }
  return 0;
}
