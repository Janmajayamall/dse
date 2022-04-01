// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import "./interfaces/IERC20.sol";
import "./libraries/Transfers.sol";

contract Wallet {

    using Transfers for IERC20;

    mapping(bytes32 => bool) claimed;
    mapping(bytes32 => uint256) burnBufferTime;
    mapping(bytes32 => uint256) burnBufferAmount;
    mapping(bytes32 => bytes32) burnBufferIBlob;
    mapping(bytes32 => address) burnBufferIAddress;

    // Wallet's on going epoch
    uint public currentEpoch;
    // Epoch's end time 
    uint public epochExpiresBy;
    // Wallet's reserves in currency
    uint public reserves;
    // ERC20 token's address used as currency by wallet
    address public currency;
    // Owner of the wallet
    address public owner;

    // Duration of a epoch
    uint immutable public epochDuration;
    // Value in currency per index
    uint immutable public indexValue;
    // Buffer time for burn
    uint immutable public burnBuffer;
    // funds to be burnt before moving to next epoch
    uint pendingBurnAmount;
    
    // Buffer time added to burn buffer time
    // of commitments in which owner isn't 
    // the invalidating address
    // This is needed to prevent attack
    // by iAddress when they prevent their funds 
    // from burning by producing signature after
    // funds of their counter party have been 
    // burnt, since buffer time is equal irrespective
    // of who is at the advantage of producing invaldating
    // signature.
    uint constant invDisadvantageBuffer = 1 hours;

    uint constant ECDSA_SIGNATURE_LENGTH = 65;

    error BalanceError();
    error NotOwner();
    error EpochOngoing();
     
    constructor(
        uint _epochDuration,
        uint _indexValue,
        address _owner,
        address _currency,
        uint _burnBuffer
    ) {
        epochDuration = _epochDuration;
        indexValue = _indexValue;
        owner = _owner;
        currency = _currency;
        burnBuffer = _burnBuffer;
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

    /// Burns pending burn amount.
    /// Note don't call this during
    /// an epoch.
    function burnPendingAmount() internal {
        uint _pendingAmount = pendingBurnAmount;
        if (_pendingAmount != 0) {
            IERC20(currency).transfer(address(0), _pendingAmount);
            pendingBurnAmount = 0;
        }
    }

    function recoverSigner(uint sIndex, bytes32 message) internal pure returns (address) {

        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(sIndex)
            s := calldataload(add(sIndex, 32))
            v := shr(248, calldataload(add(sIndex, 32)))
        }

        return ecrecover(message, v, r, s);
    }

    /// Checks whether commit of an index
    /// in the current epoch valid.
    function isValidCommitment(
        uint256 index,
        uint256 epoch
    ) public view returns (bool) {
        // zero is not a valid index
        if (index == 0) return false;

        // not current epoch or epoch expired
        if (
            epoch != currentEpoch ||
            block.timestamp >= epochExpiresBy
        ) return false;

        // check that index for the epoch hasn't been claimed 
        // nor pending burn
        bytes32 claimIdentifier = keccak256(abi.encodePacked(index, epoch));
        if (
            burnBufferTime[claimIdentifier] != 0 ||
            claimed[claimIdentifier] == true
        ) revert();
        
        // (balance / indexValue) defines index range
        // where balance = reserves / 2, since 50%
        // is used as security deposit
        if (index <= ((reserves / 2)  / indexValue)){
            return true;
        }

        return false;
    }

    /// Starts next epoch 
    function startEpoch() public {
        if (block.timestamp < epochExpiresBy) revert EpochOngoing();

        burnPendingAmount();

        reserves = getBalance(currency);
        currentEpoch += 1;
        epochExpiresBy = block.timestamp + epochDuration;

        // TODO emit event
    }

    /// Owner withdraws security deposit +
    /// not redeemed balance in the wallet
    function withdraw() public {
        if (msg.sender != owner) revert NotOwner();

        // cannot withdraw during an active epoch
        if (block.timestamp < epochDuration) revert EpochOngoing();

        burnPendingAmount();

        address token = currency;
        IERC20(token).safeTransfer(msg.sender, getBalance(token));

        reserves = 0;
    }

    /// Redeem time-locked commitments of type 2 
    /// Fn selector - 4 bytes
    /// Index count - 2 bytes
    /// To Address - 20 bytes
    /// Data per index - 139 bytes
    ///     Index(4 bytes) + U(4 bytes) + CommitSignature(65 bytes)  + InvalidatingSignature(65 bytes )
    function redeemdType2() public {

        uint16 count;
        address toAddress;
        assembly {
            count := shr(240, calldataload(4))
            toAddress := shr(96, calldataload(6))
        }

        uint256 _indexValue = indexValue;
        uint256 _currentEpoch = currentEpoch;
        address _owner = owner;

        // total amount to be redeemed
        uint256 totalAmount;

        for (uint256 i = 0; i < uint256(count); i++) {

            uint32 index;
            uint32 u;
            uint byteOffset = 26 + (139 * i); // 139 = 4 + 4 + 65 + 65
            assembly {  
                index := shr(224, calldataload(byteOffset))
                u := shr(224, calldataload(add(byteOffset, 4)))
            }

            // check that index for the epoch hasn't been claimed 
            // nor pending burn
            bytes32 claimIdentifier = keccak256(abi.encodePacked(index, _currentEpoch));
            if (
                burnBufferTime[claimIdentifier] != 0 ||
                claimed[claimIdentifier] == true
            ) revert();

            bytes32 commit = keccak256(abi.encodePacked(
                uint256(index), 
                uint256(_currentEpoch), 
                uint256(u), 
                uint256(2), 
                _owner, 
                toAddress
            ));

            // checks that owner signed the commitment
            if (recoverSigner(byteOffset + 8, commit) != _owner) revert();

            // check that owner signed the invalidating message
            if (recoverSigner(byteOffset + 73, keccak256(abi.encodePacked(u))) != _owner) revert();

            totalAmount += _indexValue;

            claimed[claimIdentifier] = true;
        }

        IERC20(currency).transfer(toAddress, totalAmount);

        // TODO emit claimType2
    }

    /// Burn commitments with no invalidating
    /// signatures
    /// Fn selector - 4 bytes
    /// Index count - 2 bytes
    /// ToAddress - 20 bytes
    /// Data per index - 74 bytes
    ///     Index(4 bytes) + U(4 bytes) + Type(1 byte) + CommitSignature(65 bytes)
    ///     Type - 
    ///         (1) CType 1 and IAddress = Owner Address
    ///         (2) CType 1 and IAddress = To Address
    ///         (3) CType 2 (IAddress is owner address by default)
    function burnCommits() public {
        uint _burnBuffer = burnBuffer;

        // Can't call burn after epochExpiry - burnBuffer - invDisadvantageBuffer.
        if (block.timestamp >= epochExpiresBy - (_burnBuffer + invDisadvantageBuffer)) revert();

        uint16 count;
        address toAddress;
        assembly {
            count := shr(240, calldataload(4))
            toAddress := shr(96, calldataload(6))
        }

        uint256 _indexValue = indexValue;
        uint256 _currentEpoch = currentEpoch;
        address _owner = owner;
        uint256 totalBurn;

        for (uint256 i = 0; i < count; i++) {
            uint32 index;
            uint32 u;
            uint8 _type;
            uint byteOffset = 26 + (74 * i); // 74 = 4 + 4 + 1 + 65
            assembly {  
                index := shr(224, calldataload(byteOffset))
                u := shr(224, calldataload(add(byteOffset, 4)))
                _type := shr(248, calldataload(add(byteOffset, 8)))
            } 

            bytes32 claimIdentifier = keccak256(abi.encodePacked(index, _currentEpoch));
            // check that index for the epoch hasn't been claimed 
            // nor pending burn
            if (
                burnBufferTime[claimIdentifier] != 0 ||
                claimed[claimIdentifier] == true
            ) revert();

            bytes32 commit;
            uint256 bTime = _burnBuffer;
            address _toAddress = toAddress;
            if (_type < 2){
                address iAddress = _type == 0 ? _owner : toAddress;
                // type 1 commits
                commit = keccak256(
                    abi.encodePacked(
                        uint256(index), 
                        uint256(_currentEpoch), 
                        uint256(u), 
                        uint256(1), 
                        iAddress
                    )
                );

                burnBufferIAddress[claimIdentifier] = iAddress;

                // if owner isn't iAddress
                // add invDisadvantageTime to buffer time
                if (_type == 1){
                    bTime += invDisadvantageBuffer;
                }
            }else {
                // type 2 commit
                // Note that type 2 commits iAddress are 
                // onwers themselves by default
                commit = keccak256(
                    abi.encodePacked(
                        uint256(index), 
                        uint256(_currentEpoch), 
                        uint256(u),  
                        uint256(2), 
                        _owner,
                        _toAddress
                    )
                );
                burnBufferIAddress[claimIdentifier] = _owner;
            }

            // checks that owner signed the commitment
            if (recoverSigner(byteOffset + 9, commit) != _owner) revert();

            burnBufferTime[claimIdentifier] = bTime;
            burnBufferIBlob[claimIdentifier] = keccak256(abi.encodePacked(u));

            totalBurn += _indexValue;
        }

        pendingBurnAmount = pendingBurnAmount + totalBurn;

        // TODO emit event
    }

    /// Challenge commit in burn buffer 
    /// by providing invalidating signatures.
    function challengeBurnCommits(
        bytes32[] calldata commitHash,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s
    ) public {
        // sanity checks
        if (commitHash.length != v.length) revert();
        if (
            v.length != r.length || 
            v.length != s.length
        ) revert();

        uint totalBurn = 0;
        uint _indexValue = indexValue;

        for (uint256 i = 0; i < commitHash.length; i++) {
            if (burnBufferTime[commitHash[i]] <= block.timestamp) revert();
            if (ecrecover(burnBufferIBlob[commitHash[i]], v[i], r[i], s[i]) != burnBufferIAddress[commitHash[i]]) revert();

            // challenge is valid, delete records
            delete burnBufferTime[commitHash[i]];
            delete burnBufferIBlob[commitHash[i]];
            delete burnBufferIAddress[commitHash[i]];

            totalBurn += _indexValue;
        }

        pendingBurnAmount = pendingBurnAmount - totalBurn;
    }
} 