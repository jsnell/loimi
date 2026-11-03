/* Shim for benchmark programs that read their problem size from argv[1].
   The program is compiled with -Dmain=bench_main and linked with this file,
   which supplies the size from -DBENCH_ARG=... so the .wasm needs no
   command-line arguments (and imports no args_get/args_sizes_get). */
#define STR_(x) #x
#define STR(x) STR_(x)

int bench_main(int argc, char **argv);

int main(void) {
  static char prog[] = "bench";
  static char arg[] = STR(BENCH_ARG);
  char *argv[] = {prog, arg, 0};
  return bench_main(2, argv);
}
