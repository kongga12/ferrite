package com.example.board;

public class Page {
    private int rows;

    public Page(int rows) {
        this.rows = rows;
    }

    public int count(int total) {
        return total / rows + offset(total);
    }

    private int offset(int pageNo) {
        return 1;
    }
}
