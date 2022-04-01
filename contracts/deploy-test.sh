#!/bin/bash


PV_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
ADD=0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266
RPC=http://127.0.0.1:8545

forge create Wallet --constructor-args 864000 10000000000000000 0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266 0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266 86400 --rpc-url $RPC --private-key $PV_KEY