// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import "./interfaces/IERC20.sol";
import "./libraries/Transfers.sol";

contract Wallet {

    using Transfers for IERC20;

    mapping(bytes32 => bool) claimed;
    mapping(bytes32 => uint256) burnBufferTime;
    mapping(bytes32 => uint256) burnBufferAmount;
    mapping(bytes32 => bytes32) burnBufferIPBlob;
    mapping(bytes32 => address) burnBufferIAddress;

    uint public currentEpoch;
    uint public epochExpiresBy;
    uint public securityDeposit;
    address public currency;
    address public owner;
    uint immutable public epochDuration;
    uint immutable public indexValue;
    uint immutable public bufferTime;

    error BalanceError();
    error NotOwner();
    error EpochOngoing();
     
    constructor(
        uint _epochDuration,
        uint _indexValue,
        uint _securityDeposit,
        address _owner,
        address _currency,
        uint _bufferTime
    ) {
        epochDuration = _epochDuration;
        indexValue = _indexValue;
        securityDeposit = _securityDeposit;
        owner = _owner;
        currency = _currency;
        bufferTime = _bufferTime;

        currentEpoch = 1;
        epochExpiresBy = block.timestamp + _epochDuration;
    }

    function getBalance(
        address token
    ) internal view returns (uint256 balance) {
        (bool success, bytes memory data) = token.staticcall(
            abi.encodeWithSelector(IERC20.balanceOf.selector, address(this))
        );
        if (!success || data.length != 32) revert BalanceError();
        balance = abi.decode(data, (uint256));
    }

    function isValidCommitment(
        uint256 index,
        uint256 epoch
    ) public view returns (bool isValid) {
        // zero is not an index
        if (index == 0) return false;
        // not current epoch 
        if (epoch != currentEpoch) return false;
        // epoch has expired
        if (block.timestamp >= epochExpiresBy) return false;
        
        uint balance = getBalance(address(currency));
        if (index < (balance / indexValue)){
            return true;
        }
        return false;
    }

    function deposit() public {
        uint256 _securityDeposit = securityDeposit;
        uint256 amount = getBalance(currency) - _securityDeposit;
        securityDeposit = amount + _securityDeposit;
        // TODO emit event
    }

    function withdraw() public {
        if (msg.sender != owner) revert NotOwner();
        if (block.timestamp < epochDuration) revert EpochOngoing();

        address token = currency;
        IERC20(token).safeTransfer(msg.sender, getBalance(token));

        securityDeposit = 0;
    }

    function renew() public {
        if (block.timestamp < epochDuration) revert EpochOngoing();
        currentEpoch += 1;
        epochExpiresBy = block.timestamp + epochExpiresBy;
        // TODO emit event
    }

    // claim time locked commitments of type 2 
    // that have invalidating signatures
    function claimType2(
        uint24[] calldata indexes,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s,
        uint8[] calldata vi,
        bytes32[] calldata ri,
        bytes32[] calldata si,
        address to
    ) public {
        if (block.timestamp > epochExpiresBy) revert();

        if (indexes.length / 3 != v.length) revert();
        if (v.length != r.length || v.length != s.length) revert();
        if (vi.length != ri.length || vi.length != si.length) revert();
        if (v.length != vi.length) revert();

        uint256 _indexValue = indexValue;
        uint256 epoch = currentEpoch;
        address _owner = owner;
        uint256 totalAmount;

        for (uint256 i = 0; i < v.length; i++) {
            uint256 indexStart = indexes[i * 3];
            uint256 indexEnd = indexes[i * 3 + 1];
            uint256 u = indexes[i * 3 + 2];

            bytes32 val = keccak256(abi.encodePacked(indexStart, indexEnd, epoch, u, uint256(2), to));
            // checks that owner signed the commitment
            if (ecrecover(val, v[i], r[i], s[i]) != _owner) revert();

            // check that owner signed the
            // invalidating message
            bytes32 iVal = keccak256(abi.encodePacked(u));
            if (ecrecover(iVal, vi[i], ri[i], si[i]) != _owner) revert();

            totalAmount += _indexValue * (indexEnd - indexStart);

            claimed[val] = true;

        }

        IERC20(currency).transfer(to, totalAmount);
        // TODO emit claimType2
    }

    function burn(
        uint24[] calldata indexes,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s,
        address iAddress,
        address to,
        uint256 splitIndex
    ) public {
        // Can't call burn 24 hours prior to epoch expiry.
        // TODO 24 hours should be made configurable
        if (block.timestamp > epochExpiresBy - (24 hours)) revert();

        if (indexes.length / 3 != v.length) revert();
        if (v.length != r.length || v.length != s.length) revert();

        uint256 _indexValue = indexValue;
        uint256 epoch = currentEpoch;
        address _owner = owner;
        uint _bufferTime = bufferTime;

        for (uint256 i = 0; i < v.length; i++) {
            uint256 indexStart = indexes[i * 3];
            uint256 indexEnd = indexes[i * 3 + 1];
            uint256 u = indexes[i * 3 + 2];

            bytes32 val;
            uint256 time = _bufferTime;
            if (i < splitIndex){
                // type 1
                val = keccak256(abi.encodePacked(indexStart, indexEnd, epoch, u, uint256(1), iAddress));
                burnBufferIAddress[val] = iAddress;

                // add extra time if iAddress isn't owner
                if (_owner != iAddress){
                    // TODO change this to variable
                    time += 18000;
                }
            }else {
                // type 2
                val = keccak256(abi.encodePacked(indexStart, indexEnd, epoch, u, uint256(2), to));
                burnBufferIAddress[val] = _owner;
            }

            // checks that owner signed the commitment
            if (ecrecover(val, v[i], r[i], s[i]) != _owner) revert();

            burnBufferTime[val] = time;
            burnBufferIPBlob[val] = keccak256(abi.encodePacked(u));
            burnBufferAmount[val] = (indexEnd - indexStart) * _indexValue;

        }

        // TODO emit event
    }

    function challengeBurn(
        bytes32[] calldata hashVals,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s
    ) public {
        if (block.timestamp > epochExpiresBy) revert();

        for (uint256 i = 0; i < hashVals.length; i++) {
            if (burnBufferTime[hashVals[i]] < block.timestamp) revert();
            if (ecrecover(burnBufferIPBlob[hashVals[i]], v[i], r[i], s[i]) != burnBufferIAddress[hashVals[i]]) revert();

            // challenge is valid, change amount to zero
            burnBufferAmount[hashVals[i]] = 0;
        }
    }

    function claim(
        uint256 amount,
        address by
    ) public {

    }

    function burn(
        uint256  amount,
        address by
    ) public {

    }
} 