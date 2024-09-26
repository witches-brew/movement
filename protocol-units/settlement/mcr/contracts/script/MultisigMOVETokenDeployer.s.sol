pragma solidity ^0.8.13;

import "forge-std/Script.sol";
import {MOVEToken} from "../src/token/MOVEToken.sol";
import {TransparentUpgradeableProxy} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {Helper, Safe} from "./helpers/Helper.sol";
import {Vm} from "forge-std/Vm.sol";
import {ICREATE3Factory} from "./helpers/Create3/ICREATE3Factory.sol";
import {Enum} from "@safe-smart-account/contracts/common/Enum.sol";
import {stdJson} from "forge-std/StdJson.sol";

// Script intended to be used for deploying the MOVE token from an EOA
// Utilizies existing safes and sets them as proposers and executors.
// The MOVEToken contract takes in the Movement Foundation address and sets it as its own admin for future upgrades.
// The whole supply is minted to the Movement Foundation Safe.
// The script also verifies that the token has the correct balances, decimals and permissions.
contract MultisigMOVETokenDeployer is Helper {
    using stdJson for string;
    // COMMANDS
    // mainnet
    // forge script MOVETokenDeployer --fork-url https://eth.llamarpc.com --verify --etherscan-api-key ETHERSCAN_API_KEY
    // testnet
    // forge script MOVETokenDeployer --fork-url https://eth-sepolia.api.onfinality.io/public
    // Safes should be already deployed

    bytes32 public salt = 0x0308000000000000000000000a18f8ed6e115a72d9d13b2c5578f132ce7f643a;
    bytes32 public constant DEFAULT_ADMIN_ROLE = 0x00;

    function run() external virtual {
        // load config and deployments data
        _loadExternalData();

        uint256 signer = vm.envUint("TEST_1");
        vm.startBroadcast(signer);

        // Deploy CREATE3Factory, Safes and Timelock if not deployed
        _deployDependencies();

        // This deployer solely deploys a timelock and an implementation, it leaves to multisig to execute the deployment
        // of the actual token.
        _proposeMultisigMove();

        vm.stopBroadcast();

        if (vm.isContext(VmSafe.ForgeContext.ScriptBroadcast)) {
            _writeDeployments();
        }
    }

    // •☽────✧˖°˖DANGER ZONE˖°˖✧────☾•

    function _proposeMultisigMove() internal {
        console.log("MOVE: deploying");
        MOVEToken moveImplementation = new MOVEToken();
        // genetares bytecode for CREATE3 deployment
        bytes memory create3Bytecode = abi.encodePacked(
            type(TransparentUpgradeableProxy).creationCode,
            abi.encode(
                address(moveImplementation),
                address(timelock),
                abi.encodeWithSignature(moveSignature, deployment.movementFoundationSafe, deployment.anchorage)
            )
        );
        // create bytecode the MOVE token proxy using CREATE3
        bytes memory bytecode = abi.encodeWithSignature("deploy(bytes32,bytes)", salt, create3Bytecode);

        // NOTE: digest can be used if immediately signing and executing the transaction
        // bytes32 digest = Safe(payable(deployment.movementFoundationSafe)).getTransactionHash(
        //     address(create3), 0, bytecode, Enum.Operation.Call, 0, 0, 0, ZERO, payable(ZERO), 0
        // );

        string memory json = "safeCall";
        // Serialize the relevant fields into JSON format
        json.serialize("to", address(create3));
        string memory zero = "0";
        json.serialize("value", zero);
        json.serialize("data", bytecode);
        string memory operation = "OperationType.Call";
        json.serialize("chainId", chainId);
        json.serialize("safeAddress", deployment.movementDeployerSafe);
        string memory serializedData = json.serialize("operation", operation);
        // Log the serialized JSON for debugging
        console.log("json |start|", serializedData, "|end|");
        // Write the serialized data to a file
        if (vm.isContext(VmSafe.ForgeContext.ScriptBroadcast)) {
            vm.writeFile(string.concat(root, upgradePath, "deploymove.json"), serializedData);
        }
    }

    function _deployMultisigMove() internal {
        console.log("MOVE: deploying");
        MOVEToken moveImplementation = new MOVEToken();
        // genetares bytecode for CREATE3 deployment
        bytes memory create3Bytecode = abi.encodePacked(
            type(TransparentUpgradeableProxy).creationCode,
            abi.encode(
                address(moveImplementation),
                address(timelock),
                abi.encodeWithSignature(moveSignature, deployment.movementFoundationSafe)
            )
        );
        vm.recordLogs();
        // craete bytecode the MOVE token proxy using CREATE3
        bytes memory bytecode = abi.encodeWithSignature("deploy(bytes32,bytes)", salt, create3Bytecode);
        bytes32 digest = Safe(payable(deployment.movementDeployerSafe)).getTransactionHash(
            address(create3), 0, bytecode, Enum.Operation.Call, 0, 0, 0, ZERO, payable(ZERO), 0
        );

        // three signers for the deployment (this is mocked and only works in foundry chain)
        uint256[] memory signers = new uint256[](3);
        signers[0] = vm.envUint("PRIVATE_KEY");
        signers[1] = 1;
        signers[2] = 2;

        bytes memory signatures = _generateSignatures(signers, digest);

        Safe(payable(deployment.movementFoundationSafe)).execTransaction(
            address(create3), 0, bytecode, Enum.Operation.Call, 0, 0, 0, ZERO, payable(ZERO), signatures
        );
        // moveProxy =
        console.log("MOVEToken deployment records:");
        Vm.Log[] memory logs = vm.getRecordedLogs();
        deployment.move = logs[0].emitter;
        deployment.moveAdmin = logs[logs.length - 3].emitter;
        console.log("proxy", deployment.move);
        console.log("admin", deployment.moveAdmin);
    }

    // MULTISIG WILL NEVER BE USED WITHIN THE CONTRACT PIPELINE
    function _upgradeMultisigMove() internal {
        console.log("MOVE: upgrading");
        MOVEToken newMoveImplementation = new MOVEToken();
        timelock.schedule(
            deployment.moveAdmin,
            0,
            abi.encodeWithSignature(
                "upgradeAndCall(address,address,bytes)",
                deployment.move,
                address(newMoveImplementation),
                abi.encodeWithSignature("initialize(address)", deployment.movementFoundationSafe)
            ),
            bytes32(0),
            bytes32(0),
            config.minDelay
        );
    }
}
