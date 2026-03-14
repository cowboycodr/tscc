#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

// Mango string: { char* data, int64_t len }
typedef struct {
    char* data;
    long long len;
} MgString;

void mango_print_number(double n) {
    // Print integers without decimal point, like JavaScript
    if (n == (long long)n && !isinf(n) && fabs(n) < 1e15) {
        printf("%lld", (long long)n);
    } else {
        printf("%.15g", n);
    }
}

void mango_print_string(char* data, long long len) {
    fwrite(data, 1, (size_t)len, stdout);
}

void mango_print_boolean(int b) {
    if (b) {
        printf("true");
    } else {
        printf("false");
    }
}

void mango_print_null(void) {
    printf("null");
}

void mango_print_undefined(void) {
    printf("undefined");
}

void mango_print_newline(void) {
    printf("\n");
}

MgString mango_string_concat(char* a_data, long long a_len, char* b_data, long long b_len) {
    long long new_len = a_len + b_len;
    char* new_data = (char*)malloc((size_t)(new_len + 1));
    if (!new_data) {
        fprintf(stderr, "mango: out of memory\n");
        exit(1);
    }
    memcpy(new_data, a_data, (size_t)a_len);
    memcpy(new_data + a_len, b_data, (size_t)b_len);
    new_data[new_len] = '\0';

    MgString result;
    result.data = new_data;
    result.len = new_len;
    return result;
}

MgString mango_number_to_string(double n) {
    char buf[64];
    int len;
    if (n == (long long)n && !isinf(n) && fabs(n) < 1e15) {
        len = snprintf(buf, sizeof(buf), "%lld", (long long)n);
    } else {
        len = snprintf(buf, sizeof(buf), "%.15g", n);
    }

    char* data = (char*)malloc((size_t)(len + 1));
    if (!data) {
        fprintf(stderr, "mango: out of memory\n");
        exit(1);
    }
    memcpy(data, buf, (size_t)(len + 1));

    MgString result;
    result.data = data;
    result.len = len;
    return result;
}

MgString mango_boolean_to_string(int b) {
    if (b) {
        MgString result;
        result.data = "true";
        result.len = 4;
        return result;
    } else {
        MgString result;
        result.data = "false";
        result.len = 5;
        return result;
    }
}
