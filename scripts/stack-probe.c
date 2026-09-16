// Diagnostic only: reproduce Rust 1.88's main-thread guard mapping on macOS.
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <unistd.h>
int main(void) {
    struct rlimit limit;
    getrlimit(RLIMIT_STACK, &limit);
    size_t page = (size_t)sysconf(_SC_PAGESIZE);
    uintptr_t top = (uintptr_t)pthread_get_stackaddr_np(pthread_self());
    size_t size = pthread_get_stacksize_np(pthread_self());
    uintptr_t bottom = top - size;
    if (bottom % page) bottom += page - bottom % page;
    void *mapped = mmap((void *)bottom, page, PROT_READ | PROT_WRITE,
                        MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
    int error = errno;
    printf("stack-limit=%llu max=%llu page=%zu getpagesize=%d stack-size=%zu aligned=%zu mmap-ok=%d errno=%d\n",
           (unsigned long long)limit.rlim_cur, (unsigned long long)limit.rlim_max,
           page, getpagesize(), size, (size_t)(bottom % 16384), mapped == (void *)bottom, error);
    return mapped == MAP_FAILED;
}
