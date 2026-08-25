// SPDX-License-Identifier: MIT
/*
 * State-aware SO_BINDTODEVICE / SO_BINDTOIFINDEX probe for the QEMU harness.
 *
 * The actor runs as Android app uid 10000 and issues setsockopt through an inline arm64
 * `svc`, bypassing libc.  The socket itself is created by the root parent before
 * fork, so after the actor exits a separate non-target observer (uid 10001) can
 * inspect the very same open file description.  This catches the dangerous
 * failure mode where a hook returns ENODEV after the kernel has already changed
 * sk_bound_dev_if; checking errno alone cannot distinguish that from a real
 * pre-mutation denial.
 *
 * Output is deliberately line-oriented for init.sh / init-kpm.sh:
 *
 *   BIND_NAME_RAW_ERRNO=0
 *   BIND_NAME_RAW_STATE=1
 *
 * errno is positive (0 on success).  STATE is 0 for unbound, 1 for the
 * expected interface, 2 for a different interface, or a negative errno if the
 * observation itself failed.
 */

#include <errno.h>
#include <grp.h>
#include <linux/if.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef SO_BINDTODEVICE
#define SO_BINDTODEVICE 25
#endif

#ifndef SO_BINDTOIFINDEX
#define SO_BINDTOIFINDEX 62
#endif

#ifndef __NR_setsockopt
#define __NR_setsockopt 208
#endif

#ifndef __NR_getsockopt
#define __NR_getsockopt 209
#endif

#define ACTOR_UID 10000
#define OBSERVER_UID 10001
#define AID_INET 3003

struct call_result {
	int ret;
	int err;
};

static int become_app(uid_t uid, int needs_internet)
{
	gid_t groups[] = { AID_INET };

	if (setgroups(needs_internet ? 1 : 0, groups) != 0 ||
	    setgid(uid) != 0 || setuid(uid) != 0)
		return -1;
	return 0;
}

static long raw_syscall5(long nr, long a0, long a1, long a2, long a3, long a4)
{
#if defined(__aarch64__)
	register long x0 __asm__("x0") = a0;
	register long x1 __asm__("x1") = a1;
	register long x2 __asm__("x2") = a2;
	register long x3 __asm__("x3") = a3;
	register long x4 __asm__("x4") = a4;
	register long x8 __asm__("x8") = nr;

	__asm__ volatile("svc #0"
			 : "+r"(x0)
			 : "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x8)
			 : "memory", "cc");
	return x0;
#else
#error "bind-probe is intentionally arm64-only"
#endif
}

static long raw_setsockopt(int fd, int level, int optname, const void *optval,
			   socklen_t optlen)
{
	return raw_syscall5(__NR_setsockopt, fd, level, optname, (long)optval,
			    optlen);
}

static long raw_getsockopt(int fd, int level, int optname, void *optval,
			   socklen_t *optlen)
{
	return raw_syscall5(__NR_getsockopt, fd, level, optname, (long)optval,
			    (long)optlen);
}

static int write_full(int fd, const void *buf, size_t len)
{
	const char *p = buf;

	while (len) {
		ssize_t n = write(fd, p, len);
		if (n < 0) {
			if (errno == EINTR)
				continue;
			return -1;
		}
		p += n;
		len -= (size_t)n;
	}
	return 0;
}

static int read_full(int fd, void *buf, size_t len)
{
	char *p = buf;

	while (len) {
		ssize_t n = read(fd, p, len);
		if (n < 0) {
			if (errno == EINTR)
				continue;
			return -1;
		}
		if (n == 0)
			return -1;
		p += n;
		len -= (size_t)n;
	}
	return 0;
}

static struct call_result actor_setsockopt(int fd, int optname,
					   const void *optval, socklen_t optlen)
{
	int pipefd[2];
	pid_t pid;
	struct call_result result = { .ret = -1, .err = EIO };
	int status;

	if (pipe(pipefd) != 0)
		return result;
	pid = fork();
	if (pid < 0) {
		close(pipefd[0]);
		close(pipefd[1]);
		return result;
	}
	if (pid == 0) {
		long rc;

		close(pipefd[0]);
		if (become_app(ACTOR_UID, 1) != 0) {
			result.ret = -1;
			result.err = errno;
		} else {
			rc = raw_setsockopt(fd, SOL_SOCKET, optname, optval,
					    optlen);
			if (rc < 0) {
				result.ret = -1;
				result.err = (int)-rc;
			} else {
				result.ret = (int)rc;
				result.err = 0;
			}
		}
		(void)write_full(pipefd[1], &result, sizeof(result));
		close(pipefd[1]);
		_exit(0);
	}

	close(pipefd[1]);
	if (read_full(pipefd[0], &result, sizeof(result)) != 0) {
		result.ret = -1;
		result.err = EIO;
	}
	close(pipefd[0]);
	while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
		;
	return result;
}

static int observer_name_state(int fd, const char *expected)
{
	int pipefd[2];
	pid_t pid;
	int state = -EIO;
	int status;

	if (pipe(pipefd) != 0)
		return state;
	pid = fork();
	if (pid < 0) {
		close(pipefd[0]);
		close(pipefd[1]);
		return state;
	}
	if (pid == 0) {
		char name[IFNAMSIZ] = { 0 };
		socklen_t len = sizeof(name);
		long rc;

		close(pipefd[0]);
		if (become_app(OBSERVER_UID, 0) != 0) {
			state = -errno;
		} else {
			rc = raw_getsockopt(fd, SOL_SOCKET, SO_BINDTODEVICE,
					    name, &len);
			if (rc < 0)
				state = (int)rc;
			else if (len == 0 || name[0] == '\0')
				state = 0;
			else if (strcmp(name, expected) == 0)
				state = 1;
			else
				state = 2;
		}
		(void)write_full(pipefd[1], &state, sizeof(state));
		close(pipefd[1]);
		_exit(0);
	}

	close(pipefd[1]);
	if (read_full(pipefd[0], &state, sizeof(state)) != 0)
		state = -EIO;
	close(pipefd[0]);
	while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
		;
	return state;
}

static int observer_index_state(int fd, int expected)
{
	int pipefd[2];
	pid_t pid;
	int state = -EIO;
	int status;

	if (pipe(pipefd) != 0)
		return state;
	pid = fork();
	if (pid < 0) {
		close(pipefd[0]);
		close(pipefd[1]);
		return state;
	}
	if (pid == 0) {
		int ifindex = -1;
		socklen_t len = sizeof(ifindex);
		long rc;

		close(pipefd[0]);
		if (become_app(OBSERVER_UID, 0) != 0) {
			state = -errno;
		} else {
			rc = raw_getsockopt(fd, SOL_SOCKET, SO_BINDTOIFINDEX,
					    &ifindex, &len);
			if (rc < 0)
				state = (int)rc;
			else if (ifindex == 0)
				state = 0;
			else if (ifindex == expected)
				state = 1;
			else
				state = 2;
		}
		(void)write_full(pipefd[1], &state, sizeof(state));
		close(pipefd[1]);
		_exit(0);
	}

	close(pipefd[1]);
	if (read_full(pipefd[0], &state, sizeof(state)) != 0)
		state = -EIO;
	close(pipefd[0]);
	while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
		;
	return state;
}

static void run_name_case(const char *label, const char *ifname,
			  socklen_t optlen)
{
	int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
	struct call_result result;
	int state;

	if (fd < 0) {
		printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, errno, label,
		       -errno);
		return;
	}
	result = actor_setsockopt(fd, SO_BINDTODEVICE, ifname, optlen);
	state = observer_name_state(fd, ifname);
	printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, result.err, label, state);
	close(fd);
}

static void run_index_case(const char *label, int ifindex)
{
	int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
	struct call_result result;
	int state;

	if (fd < 0) {
		printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, errno, label,
		       -errno);
		return;
	}
	result = actor_setsockopt(fd, SO_BINDTOIFINDEX, &ifindex,
				  sizeof(ifindex));
	state = observer_index_state(fd, ifindex);
	printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, result.err, label, state);
	close(fd);
}

static void run_bad_pointer_case(const char *label, const char *expected)
{
	const void *bad = (const void *)(uintptr_t)0xfffffffffffff000ULL;
	int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
	struct call_result result;
	int state;

	if (fd < 0) {
		printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, errno, label,
		       -errno);
		return;
	}
	result = actor_setsockopt(fd, SO_BINDTODEVICE, bad, 4);
	state = observer_name_state(fd, expected);
	printf("%s_ERRNO=%d\n%s_STATE=%d\n", label, result.err, label, state);
	close(fd);
}

int main(int argc, char **argv)
{
	const char *vpn_name;
	int vpn_ifindex;

	if (argc != 3) {
		fprintf(stderr, "usage: %s <vpn-ifname> <vpn-ifindex>\n",
			argv[0]);
		return 2;
	}
	vpn_name = argv[1];
	vpn_ifindex = atoi(argv[2]);
	if (vpn_ifindex <= 0) {
		fprintf(stderr, "invalid vpn ifindex: %s\n", argv[2]);
		return 2;
	}

	/* Linux accepts both lengths.  Exercise both so a hook cannot accidentally
	 * depend on callers including a trailing NUL. */
	run_name_case("BIND_NAME_RAW", vpn_name, (socklen_t)strlen(vpn_name));
	run_name_case("BIND_NAME_NUL", vpn_name,
		      (socklen_t)strlen(vpn_name) + 1);
	/* A userspace pointer whose numeric value is in the kernel range must be
	 * rejected through the uaccess path, never dereferenced by a hook. */
	run_bad_pointer_case("BIND_BADPTR", vpn_name);
	/* sock_setbindtodevice treats the unsigned syscall length as int and must
	 * reject this before a hook classifies the otherwise valid VPN name. */
	run_name_case("BIND_BADLEN", vpn_name, (socklen_t)-1);
	run_index_case("BIND_INDEX", vpn_ifindex);
	run_name_case("BIND_KEEP", "eth0", 4);
	/* Ground truth for "what does an interface that does not exist look like
	 * here": EPERM on trees that test CAP_NET_RAW before parsing the name,
	 * ENODEV on trees that resolve the name first. Hiding an interface means
	 * answering exactly this, so the zygisk backend measures the same thing at
	 * runtime (hidden_bind_errno) instead of deriving it from the version. */
	run_name_case("BIND_ABSENT", "zzvpnhideprobe", 15);
	return 0;
}
