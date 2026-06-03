// Invokes the compiled C0 source file's `main` function and prints out its return value.
// We use this wrapper file because the project doesn't have a linker, so we rely on GCC.

#include <stdio.h>
extern int _c0_main();
int main() {
  printf("%d\n", _c0_main());
  return 0;
}
