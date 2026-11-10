package com.example.board;

import java.io.FileInputStream;
import java.text.SimpleDateFormat;
import java.util.List;

public class Board {
    private String unused;
    private static String server_ip = "10.1.2.3";

    public int parse(String s) {
        try {
            return Integer.parseInt(s);
        } catch (NumberFormatException e) {
        }
        return 0;
    }

    public void print(String title) {
        System.out.println("게시판: " + title);
        try {
            FileInputStream in = new FileInputStream(title);
            in.read();
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    public String label(int kind) {
        String text = new String("글");
        switch (kind) {
            case 1:
                text = "공지";
                break;
        }
        if (text.equals(null)) {
            return new SimpleDateFormat("yyyy").format(new java.util.Date());
        }
        return text.toString();
    }
}
