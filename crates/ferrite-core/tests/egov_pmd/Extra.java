package com.example.board;

interface Codes {
    int MAX = 10;
}

public class Extra {
    private final int limit = 10;

    public synchronized void sync() {
        System.err.println(limit);
    }

    public void empty() {
    }

    public void raw() {
        throw new RuntimeException("x");
    }

    public void copy(int[] a, int[] b) {
        for (int i = 0; i < a.length; i++) {
            b[i] = a[i];
        }
    }

    public String conv(int x) {
        return new Integer(x).toString();
    }

    public boolean simp(boolean f) {
        return f == true;
    }

    public boolean misplaced(String s) {
        return s.length() > 0 && s != null;
    }
};
