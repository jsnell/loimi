import subprocess, time, os
S = os.path.dirname(os.path.abspath(__file__))
open(S + "/empty.ll", "w").write("define void @f() {\n  ret void\n}\n")
b = 9
for i in range(5):
    t = time.time()
    subprocess.run(["clang", "-O2", "-c", "-x", "ir", S + "/empty.ll", "-o", S + "/empty.o"], check=True)
    b = min(b, time.time() - t)
print("clang on an empty module: %.3f s" % b)
