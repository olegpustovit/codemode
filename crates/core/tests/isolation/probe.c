#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/utsname.h>
#include <unistd.h>
extern char **environ;
static void *thread(void *value) { return value; }
int main(void) {
    for (char **p=environ; *p; ++p) { const char *end=strchr(*p,'='); fprintf(stderr,"unexpected environment name: %.*s\n",(int)(end-*p),*p); }
    assert(environ[0] == NULL);
    struct rlimit core;
    assert(getrlimit(RLIMIT_CORE,&core)==0 && core.rlim_cur==0);
    assert(getpid() == 2);
    assert(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1);
    assert(prctl(PR_GET_SECCOMP, 0, 0, 0, 0) == 2);
    struct utsname name;
    assert(uname(&name) == 0 && strcmp(name.nodename, "codemode-worker") == 0);
    assert(socket(AF_INET, SOCK_STREAM, 0) == -1 && errno == EPERM);
    assert(socket(AF_UNIX, SOCK_STREAM, 0) == -1 && errno == EPERM);
    assert(open("/tmp/forbidden", O_WRONLY | O_CREAT, 0600) == -1 && errno == EPERM);
    assert(open("/home/snowbear/.local/share/codemode/.encryption-key", O_RDONLY) == -1 && errno == ENOENT);
    assert(access("/etc/passwd", R_OK) == -1 && errno == ENOENT);
    assert(syscall(SYS_unshare, CLONE_NEWUSER) == -1 && errno == EPERM);
    assert(fork() == -1 && errno == EPERM);
    pthread_t worker;
    void *result;
    assert(pthread_create(&worker, NULL, thread, (void *)42) == 0);
    assert(pthread_join(worker, &result) == 0 && result == (void *)42);
    puts("{\"environmentEmpty\":true,\"privatePid\":2,\"noNewPrivileges\":true,\"seccomp\":2,\"networkDenied\":true,\"hostFilesDenied\":true,\"writesDenied\":true,\"forkDenied\":true,\"threadsWork\":true}");
    fflush(stdout);
    return getchar() == '\n' ? 0 : 1;
}
