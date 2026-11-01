/* Stable-inode flock protocol shared with pm-kernel. No project bytes are read. */
#define _POSIX_C_SOURCE 200809L
#include <sys/file.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>
#include <poll.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <limits.h>
#include <math.h>
static long long now_ms(void) {
  struct timespec t; if (clock_gettime(CLOCK_REALTIME, &t)) return -1;
  return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}
static int field(const char *text, const char *name, long long *out) {
  const char *p = strstr(text, name); if (!p) return -1;
  p += strlen(name); while (*p == ' ' || *p == '\t') p++;
  if (*p++ != ':') return -1;
  char *end; errno = 0; double d = strtod(p, &end);
  if (end == p || errno || !isfinite(d) || d < 0 || d > 9007199254740991.0 || floor(d) != d) return -1;
  while (*end == ' ' || *end == '\t' || *end == '\n') end++;
  if (*end != ',' && *end != '}') return -1;
  *out = (long long)d; return 0;
}
static int write_record(int fd, long pid) {
  char text[128]; long long now = now_ms(); if (now < 0) return -1;
  int n = snprintf(text, sizeof text, "{\"pid\":%ld,\"heartbeat\":%lld}", pid, now);
  return pwrite(fd, text, (size_t)n, 0) == n && !ftruncate(fd, n) && !fsync(fd) ? 0 : -1;
}
int main(int argc, char **argv) {
  if (argc != 3) return 64;
  char *end; long owner = strtol(argv[2], &end, 10);
  if (*end || owner < 1 || owner > INT_MAX || getppid() != owner) return 64;
  int created = 1;
  int fd = open(argv[1], O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
  if (fd < 0 && errno == EEXIST) { created = 0; fd = open(argv[1], O_RDWR | O_NOFOLLOW); }
  if (fd < 0 || flock(fd, LOCK_EX | LOCK_NB)) return 73;
  struct stat info; if (fstat(fd, &info) || !S_ISREG(info.st_mode) || info.st_size > 4096) return 73;
  if (!created) {
    char text[4097] = {0}; ssize_t n = pread(fd, text, sizeof text - 1, 0);
    long long pid, heartbeat;
    if (n < 1 || field(text, "\"pid\"", &pid) || field(text, "\"heartbeat\"", &heartbeat) || pid > INT_MAX) return 73;
    int alive = pid && (!kill((pid_t)pid, 0) || errno != ESRCH);
    if (pid && (alive || now_ms() - heartbeat < 30000)) return 73;
  }
  if (write_record(fd, owner)) return 74;
  puts("ready"); fflush(stdout);
  struct pollfd input = { .fd = STDIN_FILENO, .events = POLLIN | POLLHUP };
  for (;;) {
    int result = poll(&input, 1, 5000);
    if (result < 0) { if (errno == EINTR) continue; return 74; }
    if (result > 0 || getppid() != owner) break;
    if (write_record(fd, owner)) return 74;
  }
  if (write_record(fd, 0)) return 74;
  close(fd); return 0;
}
