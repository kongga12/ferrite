package com.example.board;

import javax.crypto.spec.SecretKeySpec;

public abstract class Sample {
    private static int counter;
    private String[] items;
    private int size;

    protected Sample(int size) {
        this.size = size;
        counter = size;
    }

    public void base() {
        size++;
    }

    public void setItems(String[] items) {
        this.items = items;
    }

    public String[] getItems() {
        return items;
    }

    public int read(java.io.InputStream in) throws java.io.IOException {
        int x;
        if ((x = in.read()) > 0) {
            return x;
        }
        return 0;
    }

    public String trim(String s) {
        s = s.trim();
        return s;
    }

    public void npe() {
        throw new NullPointerException();
    }

    public boolean broken(String s) {
        return s != null || !s.isEmpty();
    }

    public void empty(int x) {
        if (x > 0) {
        }
    }

    public void named(int my_param) {
        int my_var = my_param;
        helper(my_var);
    }

    public SecretKeySpec key() {
        return new SecretKeySpec("my secret here!!".getBytes(), "AES");
    }

    public boolean blank(String s) {
        return s.trim().length() == 0;
    }

    public String buffer(int x) {
        StringBuffer sb = new StringBuffer();
        sb.append("a" + x);
        return sb.toString();
    }

    public Integer boxing() {
        Integer i = Integer.valueOf(1);
        return i;
    }

    private void helper(int unusedParam) {
        size--;
    }

    private void neverCalled() {
        size = 0;
    }

    public int paren(int x) {
        return (x);
    }

    public String valueOf(int x) {
        return "a" + String.valueOf(x);
    }
}
