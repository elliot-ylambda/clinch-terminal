#define _DARWIN_C_SOURCE

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static void fail(const char *message) {
    fprintf(stderr, "clinch updater swap: %s\n", message);
    exit(EXIT_FAILURE);
}

static void fail_errno(const char *message) {
    fprintf(stderr, "clinch updater swap: %s: %s\n", message, strerror(errno));
    exit(EXIT_FAILURE);
}

static const char *path_basename(const char *path) {
    const char *slash = strrchr(path, '/');
    return slash == NULL ? path : slash + 1;
}

static void path_parent(const char *path, char parent[PATH_MAX]) {
    const char *slash = strrchr(path, '/');
    if (slash == NULL || path[0] != '/' || slash[1] == '\0') {
        fail("paths must be absolute and name an app bundle");
    }
    size_t length = slash == path ? 1 : (size_t)(slash - path);
    if (length >= PATH_MAX) {
        fail("parent path is too long");
    }
    memcpy(parent, path, length);
    parent[length] = '\0';
}

static void validate_update_suffix(const char *name) {
    const char prefix[] = ".Clinch.app.update-";
    if (strncmp(name, prefix, sizeof(prefix) - 1) != 0 || name[sizeof(prefix) - 1] == '\0') {
        fail("unexpected staged bundle name");
    }
    for (const unsigned char *cursor = (const unsigned char *)name + sizeof(prefix) - 1;
         *cursor != '\0'; cursor++) {
        if (!(isalnum(*cursor) || *cursor == '_' || *cursor == '-')) {
            fail("unsafe staged bundle name");
        }
    }
}

static void validate_bundle(const char *path, uid_t owner) {
    struct stat metadata;
    if (lstat(path, &metadata) != 0) {
        fail_errno("could not inspect bundle");
    }
    if (S_ISLNK(metadata.st_mode) || !S_ISDIR(metadata.st_mode)) {
        fail("bundle path is not a real directory");
    }
    if (metadata.st_uid != owner) {
        fail("bundle is not owned by the current user");
    }
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fail("usage: clinch-update-swap /path/Clinch.app /path/.Clinch.app.update-ID");
    }

    const char *installed = argv[1];
    const char *staged = argv[2];
    if (installed[0] != '/' || staged[0] != '/' || strchr(installed, '\n') != NULL
        || strchr(installed, '\r') != NULL || strchr(staged, '\n') != NULL
        || strchr(staged, '\r') != NULL) {
        fail("unsafe path");
    }
    if (strcmp(path_basename(installed), "Clinch.app") != 0) {
        fail("unexpected installed bundle name");
    }
    validate_update_suffix(path_basename(staged));

    char installed_parent[PATH_MAX];
    char staged_parent[PATH_MAX];
    char resolved_parent[PATH_MAX];
    path_parent(installed, installed_parent);
    path_parent(staged, staged_parent);
    if (strcmp(installed_parent, staged_parent) != 0) {
        fail("bundles are not in the same directory");
    }
    if (realpath(installed_parent, resolved_parent) == NULL) {
        fail_errno("could not resolve bundle parent");
    }
    if (strcmp(installed_parent, resolved_parent) != 0) {
        fail("bundle parent contains a symbolic link or non-canonical component");
    }

    uid_t owner = geteuid();
    validate_bundle(installed, owner);
    validate_bundle(staged, owner);

    if (renamex_np(installed, staged, RENAME_SWAP) != 0) {
        fail_errno("atomic bundle exchange failed");
    }

    int parent_fd = open(resolved_parent, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (parent_fd >= 0) {
        if (fsync(parent_fd) != 0 && errno != EINVAL) {
            fprintf(stderr, "clinch updater swap: warning: could not sync bundle parent: %s\n",
                    strerror(errno));
        }
        close(parent_fd);
    }
    return EXIT_SUCCESS;
}
